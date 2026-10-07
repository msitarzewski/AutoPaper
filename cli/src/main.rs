//! `autopaper` — the developer CLI over the AutoPaper core (like AudioPaper's apctl).
//!
//! Keys come from the repository's `.env` (OPENAI_API_KEY, GEMINI_API_KEY, OPENAI_COMPATIBLE_API_KEY), held in
//! memory only; the apps keep keys in the platform's secure store instead. Data lives in
//! `<repo>/.scratch/cli-data` unless `--data-dir` says otherwise. Run `autopaper help` for the commands.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use autopaper_core::embed::{CandleEmbedder, HashingEmbedder, cosine};
use autopaper_core::engine::{ComposePreview, Deps, Engine, EngineConfig};
use autopaper_core::model::*;
use autopaper_core::novelty::Calibration;
use autopaper_core::ports::{Embedder, ProgressDetailObserver, ProgressObserver, SecretStore};
use autopaper_core::store::Store;
use autopaper_core::testing::{FixedClock, StubHttp};
use autopaper_core::{AutoPaperError, Shown};

const REPO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
const DAY: i64 = 86_400;
/// 2026-01-01T00:00:00Z: where simulations start.
const SIMULATION_START: i64 = 1_767_225_600;
/// The apps' note when `keywords_are_narrow` (docs/app-spec.md).
const NARROW_NOTE: &str =
    "Your keywords are narrow, so new ideas are getting hard to find. Add some Maybes or raise Surprise for more variety.";

const USAGE: &str = "\
autopaper — developer CLI over the AutoPaper core

USAGE: autopaper [--data-dir DIR] [--model-dir DIR] [--env-file FILE] [--display WxH] <command> [args]

  --display WxH                           The primary display's size in pixels, as a host passes it with
                                          set_display_hint (default: the engine's 3840x2160)

COMMANDS
  moods [list]                            The moods in order, the current one marked, each with its keywords
  moods add <name> [--copy <mood>]        A new mood (a duplicate of <mood> with --copy); not made current
  moods use <mood>                        Make it the current mood (makes nothing by itself)
  moods rename <mood> --to <name>
  moods move <mood> <position>            Position from 1
  moods delete <mood>
  keywords [list]                         List the current mood's keywords in order
  keywords add <text> --must|--maybe|--avoid
  keywords weight <text> <must|maybe|avoid>
  keywords remove <text>
  settings [show]                         Show settings as JSON
  settings set <field> <value>            e.g. surprise 0.6 · cadence weekly · text_provider.kind open_ai
                                          (value is JSON if it parses, else a string; nested fields with dots)
  compose                                 Dry run: the candidates with checks, novelty and taste scores,
                                          and which would be chosen (one text call, no image)
  generate                                Make a wallpaper now (prints the progress detail a host shows)
  run                                     What the host timer does: make one if due (or revisit a liked one)
  history [--liked|--disliked|--echoes] [--mood <mood>] [--limit N]
  show <id>                               One wallpaper in full (ids as `history` prints them, or any unique
                                          prefix)
  like <id> · dislike <id>                Rate (and say whether a host would replace it now)
  echo <id>                               Make an echo of a past wallpaper
  render <id> <width> <height>            Render for a display; prints the JPEG's path
  taste                                   What has been learned
  spend                                   This month's estimated spend and estimates
  estimate [images|concepts] [WxH]        How long the chosen painter (default) or writer takes here, from the
                                          calls recorded on this computer (none until one has finished)
  models [images|concepts]                The models the chosen painting (default) or writing provider offers,
                                          as Settings lists them: id, then the name shown
  simulate --days N [--seed S] [--real-model]
                                          The agent over N simulated days with Demo providers, a fake clock and
                                          random likes/dislikes (the hashing embedder unless --real-model);
                                          uses a temporary data directory

ENVIRONMENT
  AUTOPAPER_DEMO_DELAY_SECS=N             Demo's slow mode (1–3600 s a wallpaper, reported as 8 steps), for
                                          testing progress UI without ComfyUI; off by default
";

/// `println!` for output that may hold provider or database text (titles, summaries, model names, errors):
/// control characters other than line breaks are printed as spaces, so nothing a server sends can drive the
/// terminal (OSC 52 clipboard writes, cursor moves that hide or forge earlier output).
macro_rules! say {
    () => {
        println!()
    };
    ($($arg:tt)*) => {
        println!("{}", printable(&format!($($arg)*)))
    };
}

/// `text` with control characters other than `\n` as spaces.
fn printable(text: &str) -> String {
    text.chars().map(|c| if c.is_control() && c != '\n' { ' ' } else { c }).collect()
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(args).await {
        Ok(()) => {}
        Err(message) => {
            eprintln!("autopaper: {}", printable(&message));
            std::process::exit(1);
        }
    }
}

type CliResult<T = ()> = Result<T, String>;

fn err(error: AutoPaperError) -> String {
    match error {
        AutoPaperError::MissingKey { provider } => {
            let variable = match provider {
                ProviderKind::OpenAi => "OPENAI_API_KEY",
                ProviderKind::Google => "GEMINI_API_KEY",
                _ => "OPENAI_COMPATIBLE_API_KEY",
            };
            format!("no API key for {provider:?}: set {variable} in .env")
        }
        other => other.to_string(),
    }
}

/// Global options, taken out of the arguments wherever they appear.
struct Options {
    data_dir: PathBuf,
    model_dir: PathBuf,
    env_file: PathBuf,
    /// `--display WxH`: passed to `set_display_hint`.
    display: Option<(u32, u32)>,
}

/// "4112x2658" (or "4112×2658") → (4112, 2658).
fn parse_display(value: &str) -> CliResult<(u32, u32)> {
    let size = value.split_once(['x', 'X', '×']).and_then(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?)));
    size.ok_or_else(|| format!("--display wants WIDTHxHEIGHT in pixels, e.g. 4112x2658 (got {value:?})"))
}

fn take_flag(args: &mut Vec<String>, flag: &str) -> bool {
    match args.iter().position(|arg| arg == flag) {
        Some(index) => {
            args.remove(index);
            true
        }
        None => false,
    }
}

fn take_value(args: &mut Vec<String>, flag: &str) -> CliResult<Option<String>> {
    let Some(index) = args.iter().position(|arg| arg == flag) else { return Ok(None) };
    if index + 1 >= args.len() {
        return Err(format!("{flag} needs a value"));
    }
    let value = args.remove(index + 1);
    args.remove(index);
    Ok(Some(value))
}

async fn run(mut args: Vec<String>) -> CliResult {
    let options = Options {
        data_dir: take_value(&mut args, "--data-dir")?
            .map(PathBuf::from)
            .unwrap_or_else(|| Path::new(REPO).join(".scratch/cli-data")),
        model_dir: take_value(&mut args, "--model-dir")?
            .map(PathBuf::from)
            .unwrap_or_else(|| Path::new(REPO).join("models/bge-small-en-v1.5")),
        env_file: take_value(&mut args, "--env-file")?
            .map(PathBuf::from)
            .unwrap_or_else(|| Path::new(REPO).join(".env")),
        display: take_value(&mut args, "--display")?.as_deref().map(parse_display).transpose()?,
    };
    if args.is_empty() || matches!(args[0].as_str(), "help" | "--help" | "-h") {
        print!("{USAGE}");
        return Ok(());
    }
    let command = args.remove(0);
    if command == "simulate" {
        return simulate(&options, args).await;
    }
    let engine = open(&options)?;
    match command.as_str() {
        "moods" => moods(&engine, args),
        "keywords" => keywords(&engine, args),
        "settings" => settings(&engine, args),
        "compose" => compose(&engine).await,
        "generate" => generate(&engine).await,
        "run" => run_if_due(&engine).await,
        "history" => history(&engine, args),
        "show" => show(&engine, args),
        "like" => rate(&engine, args, Rating::Liked),
        "dislike" => rate(&engine, args, Rating::Disliked),
        "echo" => echo(&engine, args).await,
        "render" => render(&engine, args),
        "taste" => taste(&engine),
        "spend" => spend(&engine),
        "models" => models(&engine, args).await,
        "estimate" => estimate(&engine, args),
        other => Err(format!("unknown command {other:?} (try `autopaper help`)")),
    }
}

fn open(options: &Options) -> CliResult<Arc<Engine>> {
    let secrets = Arc::new(EnvSecrets::load(&options.env_file)?);
    let config = EngineConfig {
        data_dir: options.data_dir.to_string_lossy().into_owned(),
        model_dir: options.model_dir.to_string_lossy().into_owned(),
        locale: locale(),
        client: "cli".into(),
    };
    let engine = Engine::open(config, secrets).map_err(err)?;
    if let Some((width, height)) = options.display {
        engine.set_display_hint(width, height).map_err(err)?;
    }
    engine.set_progress_detail_observer(Some(Arc::new(PrintDetail::default())));
    Ok(engine)
}

/// BCP 47 from LANG ("de_DE.UTF-8" → "de-DE"); "en-US" when unset or "C".
fn locale() -> String {
    let lang = std::env::var("LANG").unwrap_or_default();
    let tag = lang.split('.').next().unwrap_or("").replace('_', "-");
    if tag.is_empty() || tag == "C" || tag == "POSIX" { "en-US".into() } else { tag }
}

// ── Keys from .env ──────────────────────────────────────────────────────────────────────────

/// Secrets in memory, read from `.env` (KEY=VALUE lines; `#` comments, `export`, quotes allowed). Writes
/// from the core stay in memory.
#[derive(Default)]
struct EnvSecrets(Mutex<HashMap<String, String>>);

impl EnvSecrets {
    const VARIABLES: [(&'static str, &'static str); 2] = [("OPENAI_API_KEY", "openai.api_key"), ("GEMINI_API_KEY", "google.api_key")];

    fn load(path: &Path) -> CliResult<Self> {
        let secrets = Self::default();
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(secrets),
            Err(error) => return Err(format!("can't read {}: {error}", path.display())),
        };
        let pairs = parse_env(&text);
        // The OpenAI-compatible key belongs to the server named by OPENAI_COMPATIBLE_BASE_URL.
        let compatible_base = pairs.iter().find(|(key, _)| key == "OPENAI_COMPATIBLE_BASE_URL").map(|(_, value)| value.clone());
        for (key, value) in pairs {
            let account = if key == "OPENAI_COMPATIBLE_API_KEY" {
                autopaper_core::secret_account_for(autopaper_core::ProviderSelection {
                    kind: autopaper_core::ProviderKind::OpenAiCompatible,
                    model: String::new(),
                    base_url: compatible_base.clone(),
                })
            } else {
                Self::VARIABLES.iter().find(|(variable, _)| *variable == key).map(|(_, account)| account.to_string())
            };
            if let Some(account) = account {
                secrets.set(account, value);
            }
        }
        Ok(secrets)
    }
}

impl SecretStore for EnvSecrets {
    fn get(&self, account: String) -> Option<String> {
        self.0.lock().unwrap().get(&account).cloned()
    }

    fn set(&self, account: String, value: String) {
        self.0.lock().unwrap().insert(account, value);
    }

    fn delete(&self, account: String) {
        self.0.lock().unwrap().remove(&account);
    }
}

/// KEY=VALUE pairs with non-empty values.
fn parse_env(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let line = line.strip_prefix("export ").unwrap_or(line);
            let (key, value) = line.split_once('=')?;
            let value = value.trim();
            let value = ['"', '\'']
                .iter()
                .find_map(|quote| value.strip_prefix(*quote).and_then(|v| v.strip_suffix(*quote)))
                .unwrap_or_else(|| value.split(" #").next().unwrap_or(value).trim());
            (!value.is_empty()).then(|| (key.trim().to_string(), value.to_string()))
        })
        .collect()
}

// ── Keywords and settings ───────────────────────────────────────────────────────────────────

fn weight_name(weight: KeywordWeight) -> &'static str {
    match weight {
        KeywordWeight::Must => "Must",
        KeywordWeight::Maybe => "Maybe",
        KeywordWeight::Avoid => "Avoid",
    }
}

fn parse_weight(text: &str) -> CliResult<KeywordWeight> {
    match text.trim_start_matches('-').to_ascii_lowercase().as_str() {
        "must" => Ok(KeywordWeight::Must),
        "maybe" => Ok(KeywordWeight::Maybe),
        "avoid" => Ok(KeywordWeight::Avoid),
        other => Err(format!("{other:?} isn't a weight (must, maybe or avoid)")),
    }
}

/// A mood by name (case-insensitive), or a unique prefix of one.
fn find_mood(engine: &Engine, name: &str) -> CliResult<Mood> {
    let wanted = name.trim().to_lowercase();
    let moods = engine.moods().map_err(err)?;
    if let Some(mood) = moods.iter().find(|mood| mood.name.to_lowercase() == wanted) {
        return Ok(mood.clone());
    }
    let matches: Vec<&Mood> = moods.iter().filter(|mood| mood.name.to_lowercase().starts_with(&wanted)).collect();
    match matches.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(format!("no mood {name:?}")),
        _ => Err(format!("{name:?} matches {} moods", matches.len())),
    }
}

/// "rain · beach · no people", as the apps list a mood's keywords.
fn keywords_in_short(keywords: &[Keyword]) -> String {
    let words: Vec<String> = keywords
        .iter()
        .map(|keyword| if keyword.weight == KeywordWeight::Avoid { format!("no {}", keyword.text) } else { keyword.text.clone() })
        .collect();
    if words.is_empty() { "no keywords yet".to_string() } else { words.join(" · ") }
}

fn moods(engine: &Engine, mut args: Vec<String>) -> CliResult {
    let action = if args.is_empty() { "list".to_string() } else { args.remove(0) };
    match action.as_str() {
        "list" => {
            for mood in engine.moods().map_err(err)? {
                let current = if mood.active { "✓" } else { " " };
                say!(
                    "{current} {}. {} — {} · Surprise {:.0}%{}",
                    mood.position + 1,
                    mood.name,
                    keywords_in_short(&mood.keywords),
                    mood.surprise * 100.0,
                    if mood.active { " (current)" } else { "" }
                );
            }
            Ok(())
        }
        "add" => {
            let copy = take_value(&mut args, "--copy")?.map(|name| find_mood(engine, &name)).transpose()?;
            let mood = engine.create_mood(args.join(" "), copy.map(|mood| mood.id)).map_err(err)?;
            say!("Added {} — {}", mood.name, keywords_in_short(&mood.keywords));
            Ok(())
        }
        "use" => {
            let mood = find_mood(engine, &args.join(" "))?;
            engine.set_active_mood(mood.id).map_err(err)?;
            say!("Current mood: {}", mood.name);
            Ok(())
        }
        "rename" => {
            let name = take_value(&mut args, "--to")?.ok_or("moods rename <mood> --to <name>")?;
            let mood = find_mood(engine, &args.join(" "))?;
            let renamed = engine.rename_mood(mood.id, name).map_err(err)?;
            say!("{} is now {}", mood.name, renamed.name);
            Ok(())
        }
        "move" => {
            let position: u32 = args.pop().and_then(|n| n.parse().ok()).ok_or("moods move <mood> <position>")?;
            let mood = find_mood(engine, &args.join(" "))?;
            engine.move_mood(mood.id, position.saturating_sub(1)).map_err(err)?;
            moods(engine, Vec::new())
        }
        "delete" => {
            let mood = find_mood(engine, &args.join(" "))?;
            engine.delete_mood(mood.id).map_err(err)?;
            say!("Deleted {}. Current mood: {}", mood.name, engine.active_mood().map_err(err)?.name);
            Ok(())
        }
        other => Err(format!("unknown moods action {other:?}")),
    }
}

fn find_keyword(engine: &Engine, text: &str) -> CliResult<Keyword> {
    let wanted = text.trim().to_lowercase();
    engine
        .keywords()
        .map_err(err)?
        .into_iter()
        .find(|keyword| keyword.text.to_lowercase() == wanted)
        .ok_or_else(|| format!("no keyword {text:?}"))
}

fn keywords(engine: &Engine, mut args: Vec<String>) -> CliResult {
    let action = if args.is_empty() { "list".to_string() } else { args.remove(0) };
    match action.as_str() {
        "list" => {
            let list = engine.keywords().map_err(err)?;
            if list.is_empty() {
                say!("No keywords yet. Add one: autopaper keywords add \"rain\" --must");
            }
            for keyword in list {
                say!("{:>2}. {:<6} {}", keyword.position + 1, weight_name(keyword.weight), keyword.text);
            }
            if engine.keywords_are_narrow().map_err(err)? {
                say!();
                say!("{NARROW_NOTE}");
            }
            Ok(())
        }
        "add" => {
            let weight = ["--must", "--maybe", "--avoid"]
                .iter()
                .find(|flag| take_flag(&mut args, flag))
                .map(|flag| parse_weight(flag))
                .transpose()?
                .unwrap_or(KeywordWeight::Maybe);
            let text = args.join(" ");
            let keyword = engine.add_keyword(text, weight).map_err(err)?;
            say!("{} · {}", keyword.text, weight_name(keyword.weight));
            Ok(())
        }
        "weight" => {
            let weight = parse_weight(args.last().ok_or("keywords weight <text> <must|maybe|avoid>")?)?;
            args.pop();
            let keyword = find_keyword(engine, &args.join(" "))?;
            engine.set_keyword_weight(keyword.id, weight).map_err(err)?;
            say!("{} · {}", keyword.text, weight_name(weight));
            Ok(())
        }
        "remove" => {
            let keyword = find_keyword(engine, &args.join(" "))?;
            engine.remove_keyword(keyword.id).map_err(err)?;
            say!("Removed {}", keyword.text);
            Ok(())
        }
        other => Err(format!("unknown keywords action {other:?}")),
    }
}

fn settings(engine: &Engine, mut args: Vec<String>) -> CliResult {
    let action = if args.is_empty() { "show".to_string() } else { args.remove(0) };
    let current = engine.settings().map_err(err)?;
    match action.as_str() {
        "show" => {
            say!("{}", serde_json::to_string_pretty(&current).map_err(|e| e.to_string())?);
            Ok(())
        }
        "set" => {
            if args.len() < 2 {
                return Err("settings set <field> <value>".into());
            }
            let field = args.remove(0);
            let raw = args.join(" ");
            let value = serde_json::from_str::<serde_json::Value>(&raw).unwrap_or(serde_json::Value::String(raw));
            let mut json = serde_json::to_value(&current).map_err(|e| e.to_string())?;
            let pointer = format!("/{}", field.replace('.', "/"));
            let slot = json.pointer_mut(&pointer).ok_or_else(|| format!("no setting {field:?}"))?;
            *slot = value;
            let updated: Settings =
                serde_json::from_value(json).map_err(|error| format!("{field}: that value doesn't fit ({error})"))?;
            engine.update_settings(updated).map_err(err)?;
            // Through the string form: `to_value` widens f32 to f64 (0.6 → 0.6000000238418579).
            let saved_text = serde_json::to_string(&engine.settings().map_err(err)?).map_err(|e| e.to_string())?;
            let saved: serde_json::Value = serde_json::from_str(&saved_text).map_err(|e| e.to_string())?;
            say!("{field} = {}", saved.pointer(&pointer).cloned().unwrap_or_default());
            Ok(())
        }
        other => Err(format!("unknown settings action {other:?}")),
    }
}

// ── Making ──────────────────────────────────────────────────────────────────────────────────

struct PrintStages;

impl ProgressObserver for PrintStages {
    fn on_progress(&self, stage: ProgressStage) {
        let label = match stage {
            ProgressStage::Composing => "Composing…",
            ProgressStage::CheckingMemory => "Checking memory…",
            ProgressStage::Generating => "Painting…",
            ProgressStage::Downloading => "Receiving…",
            ProgressStage::Rendering => "Storing…",
            ProgressStage::Done => "Done.",
        };
        eprintln!("  {label}");
    }
}

fn observer() -> Option<Arc<dyn ProgressObserver>> {
    Some(Arc::new(PrintStages))
}

/// Prints a painting's fraction and time left as a host would show them, when they move on by 5 points or the
/// minutes left change.
#[derive(Default)]
struct PrintDetail(Mutex<Option<(u32, Option<u32>)>>);

impl ProgressDetailObserver for PrintDetail {
    fn on_progress_detail(&self, detail: ProgressDetail) {
        if detail.stage != ProgressStage::Generating {
            *self.0.lock().unwrap() = None;
            return;
        }
        let Some(fraction) = detail.fraction else { return };
        let percent = (fraction * 100.0).floor() as u32;
        let left = detail.seconds_left.map(|seconds| seconds.div_ceil(60));
        let mut said = self.0.lock().unwrap();
        if said.is_some_and(|(before, minutes)| percent < before + 5 && minutes == left) {
            return;
        }
        *said = Some((percent, left));
        let time = match detail.seconds_left {
            Some(0) => " · finishing".to_string(),
            Some(seconds) if seconds < 60 => format!(" · about {seconds} s left"),
            Some(seconds) => format!(" · about {} min left", seconds.div_ceil(60)),
            None => String::new(),
        };
        eprintln!("    {percent}%{time}");
    }
}

async fn compose(engine: &Engine) -> CliResult {
    let started = Instant::now();
    let preview: ComposePreview = engine.compose_preview().await.map_err(err)?;
    say!(
        "{} candidates from {} in {:.1}s · memory: {} (too similar at ≥ {:.2}) · cost {}",
        preview.candidates.len(),
        preview.text_model,
        started.elapsed().as_secs_f32(),
        preview.embedding_model,
        preview.threshold,
        money(preview.cost_microusd)
    );
    for (index, candidate) in preview.candidates.iter().enumerate() {
        let marker = if preview.chosen == Some(index) { "→" } else { " " };
        say!();
        say!("{marker} {}. {}", index + 1, candidate.concept.title);
        say!("     {}", candidate.concept.summary);
        let checks = if candidate.problems.is_empty() { "pass".to_string() } else { candidate.problems.join("; ") };
        say!("     checks: {checks}");
        let nearest = candidate.nearest.as_deref().map(|title| format!(" (closest: “{title}”)")).unwrap_or_default();
        say!(
            "     novelty: similarity {:.3}{nearest} → {} · taste {:+.3} · score {:.3}",
            candidate.similarity,
            if candidate.novel { "novel" } else { "too similar" },
            candidate.taste,
            candidate.score
        );
        say!("     prompt: {}", candidate.concept.prompt);
    }
    say!();
    match preview.chosen {
        Some(index) => say!("Would choose #{}.", index + 1),
        None => say!("No candidate is usable; generate would ask again."),
    }
    Ok(())
}

fn print_generation(generation: &Generation) {
    say!("{}  {}", short(&generation.id), generation.concept.title);
    say!("  {}", generation.concept.summary);
    if let Some(note) = &generation.echo_note {
        say!("  {note}");
    }
    say!(
        "  {} · {}×{} · {} / {} · {} · mood: {}",
        date(generation.created_at),
        generation.width,
        generation.height,
        generation.text_model,
        generation.image_model,
        money(generation.cost_microusd),
        generation.mood_name.as_deref().unwrap_or("—")
    );
    if let Some(path) = &generation.image_path {
        say!("  {path}");
    }
}

async fn generate(engine: &Engine) -> CliResult {
    let generation = engine.generate(Trigger::Manual, observer()).await.map_err(err)?;
    print_generation(&generation);
    if engine.keywords_are_narrow().map_err(err)? {
        say!("{NARROW_NOTE}");
    }
    Ok(())
}

async fn run_if_due(engine: &Engine) -> CliResult {
    match engine.run_if_due(observer()).await.map_err(err)? {
        None => {
            match (engine.next_due().map_err(err)?, engine.next_start().map_err(err)?) {
                (Some(due), Some(start)) if start < due => {
                    say!("Nothing due until {} (it starts at {}, to be ready on time).", datetime(due), datetime(start))
                }
                (Some(due), _) => say!("Nothing due until {}.", datetime(due)),
                (None, _) => say!("Nothing due (paused, or the cadence is manual)."),
            }
            Ok(())
        }
        Some(Shown { generation, revisit }) => {
            if let Some(reason) = revisit {
                say!("Revisiting a liked wallpaper ({reason:?}):");
            }
            print_generation(&generation);
            Ok(())
        }
    }
}

async fn echo(engine: &Engine, args: Vec<String>) -> CliResult {
    let id = resolve(engine, args.first().ok_or("echo <id>")?)?;
    let generation = engine.make_echo(id, observer()).await.map_err(err)?;
    print_generation(&generation);
    Ok(())
}

// ── History and rating ──────────────────────────────────────────────────────────────────────

/// A full id, or a unique prefix of one in history, or of its last block (the short id `history` prints).
fn resolve(engine: &Engine, id: &str) -> CliResult<String> {
    if engine.generation(id.to_string()).is_ok() {
        return Ok(id.to_string());
    }
    let matches: Vec<String> = engine
        .history(HistoryFilter::All, 100_000, 0)
        .map_err(err)?
        .into_iter()
        .map(|generation| generation.id)
        .filter(|candidate| {
            candidate.starts_with(id) || candidate.rsplit('-').next().is_some_and(|tail| tail.starts_with(id))
        })
        .collect();
    match matches.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(format!("no wallpaper {id:?}")),
        _ => Err(format!("{id:?} matches {} wallpapers; give more of the id", matches.len())),
    }
}

fn history(engine: &Engine, mut args: Vec<String>) -> CliResult {
    let filter = if take_flag(&mut args, "--liked") {
        HistoryFilter::Liked
    } else if take_flag(&mut args, "--disliked") {
        HistoryFilter::Disliked
    } else if take_flag(&mut args, "--echoes") {
        HistoryFilter::Echoes
    } else {
        HistoryFilter::All
    };
    let limit = take_value(&mut args, "--limit")?.map(|n| n.parse::<u32>().map_err(|e| e.to_string())).transpose()?;
    let mood = take_value(&mut args, "--mood")?.map(|name| find_mood(engine, &name)).transpose()?;
    let list = engine.history_by_mood(filter, mood.map(|mood| mood.id), limit.unwrap_or(30), 0).map_err(err)?;
    if list.is_empty() {
        say!("Nothing here yet.");
    }
    for generation in list {
        let rating = match generation.rating {
            Rating::Liked => "♥",
            Rating::Disliked => "✕",
            Rating::Unrated => " ",
        };
        let pruned = if generation.image_path.is_none() { " (image pruned)" } else { "" };
        say!(
            "{} {} {rating} {}{pruned}",
            short(&generation.id),
            date(generation.created_at),
            generation.concept.title
        );
        if let Some(note) = &generation.echo_note {
            say!("                     {note}");
        }
    }
    Ok(())
}

fn show(engine: &Engine, args: Vec<String>) -> CliResult {
    let id = resolve(engine, args.first().ok_or("show <id>")?)?;
    let generation = engine.generation(id.clone()).map_err(err)?;
    print_generation(&generation);
    let concept = &generation.concept;
    say!("  setting: {} · subject: {}", concept.setting, concept.subject);
    say!("  elements: {}", concept.elements.join(", "));
    say!(
        "  {} · {} · {} · mood: {}",
        concept.time_of_day,
        concept.weather,
        concept.season,
        concept.mood.join(", ")
    );
    say!("  palette: {} · style: {}", concept.palette.join(", "), concept.style);
    say!("  composition: {}", concept.composition);
    say!("  keywords used: {} · wildcards: {}", concept.keywords_used.join(", "), concept.wildcards.join(", "));
    say!("  prompt: {}", concept.prompt);
    say!("  id: {id}");
    say!("  spoken: {}", engine.describe(id).map_err(err)?);
    Ok(())
}

fn rate(engine: &Engine, args: Vec<String>, rating: Rating) -> CliResult {
    let id = resolve(engine, args.first().ok_or("like|dislike <id>")?)?;
    let replace = engine.rate(id.clone(), rating).map_err(err)?;
    let title = engine.generation(id).map_err(err)?.concept.title;
    let verb = if rating == Rating::Liked { "Liked" } else { "Disliked" };
    say!("{verb} “{title}”.");
    if replace {
        say!("It's the one showing: a host would replace it now (autopaper generate).");
    }
    Ok(())
}

fn render(engine: &Engine, args: Vec<String>) -> CliResult {
    let [id, width, height] = args.as_slice() else { return Err("render <id> <width> <height>".into()) };
    let id = resolve(engine, id)?;
    let width: u32 = width.parse().map_err(|_| format!("{width:?} isn't a width"))?;
    let height: u32 = height.parse().map_err(|_| format!("{height:?} isn't a height"))?;
    let display = DisplayTarget { id: "cli".into(), width, height };
    say!("{}", engine.render_for_display(id, display).map_err(err)?);
    Ok(())
}

fn taste(engine: &Engine) -> CliResult {
    let summary = engine.taste_summary().map_err(err)?;
    say!("{} ratings", summary.ratings);
    say!("Tends to like: {}", list_or_none(&summary.liked));
    say!("Tends to dislike: {}", list_or_none(&summary.disliked));
    Ok(())
}

/// What a host's model picker lists for the chosen provider (`list_models`): id, then the name shown.
async fn models(engine: &Engine, args: Vec<String>) -> CliResult {
    let job = match args.first().map(String::as_str) {
        None | Some("images") => ProviderJob::Images,
        Some("concepts") => ProviderJob::Concepts,
        Some(other) => return Err(format!("models [images|concepts], not {other:?}")),
    };
    let settings = engine.settings().map_err(err)?;
    let selection = if job == ProviderJob::Images { settings.image_provider } else { settings.text_provider };
    let default = autopaper_core::providers::registry::default_model(selection.kind, job);
    let models = engine.list_models(selection.clone(), job).await.map_err(err)?;
    say!("{:?}: {} model{} (default {})", selection.kind, models.len(), if models.len() == 1 { "" } else { "s" }, if default.is_empty() { "none" } else { &default });
    for model in models {
        let chosen = if model.id == selection.model { "→" } else { " " };
        say!("{chosen} {}  {}", model.id, model.display_name);
    }
    Ok(())
}

/// `Engine::estimate` for the chosen painter or writer, as Settings would say it.
fn estimate(engine: &Engine, mut args: Vec<String>) -> CliResult {
    let size = args.iter().position(|arg| arg.contains(['x', 'X', '×'])).map(|index| args.remove(index));
    let (width, height) = size.as_deref().map(parse_display).transpose()?.unwrap_or((0, 0));
    let job = match args.first().map(String::as_str) {
        None | Some("images") => ProviderJob::Images,
        Some("concepts") => ProviderJob::Concepts,
        Some(other) => return Err(format!("estimate [images|concepts] [WxH], not {other:?}")),
    };
    let settings = engine.settings().map_err(err)?;
    let selection = if job == ProviderJob::Images { settings.image_provider } else { settings.text_provider };
    let model = if selection.model.is_empty() { "default model".to_string() } else { selection.model.clone() };
    let size = match (job, width) {
        (ProviderJob::Concepts, _) => String::new(),
        (_, 0) => ", the size it would ask for".to_string(),
        _ => format!(", {width}×{height}"),
    };
    match engine.estimate(selection.clone(), job, width, height).map_err(err)? {
        Some(seconds) => say!("{:?} ({model}){size}: about {} s on this computer", selection.kind, seconds),
        None => say!("{:?} ({model}): nothing recorded on this computer yet", selection.kind),
    }
    Ok(())
}

fn spend(engine: &Engine) -> CliResult {
    let summary = engine.spend_summary().map_err(err)?;
    let budget = summary.budget_cents.map_or("no cap".to_string(), |cents| money(u64::from(cents) * 10_000));
    say!(
        "{}: {} estimated, {} images (budget {budget})",
        summary.month,
        money(summary.spent_microusd),
        summary.images
    );
    say!("Next wallpaper: about {}", money(summary.per_image_microusd));
    say!("A month at this cadence: about {}", money(summary.monthly_estimate_microusd));
    Ok(())
}

// ── simulate ────────────────────────────────────────────────────────────────────────────────

/// xorshift64*: the simulated person's likes and dislikes, reproducible from the seed.
struct Person(u64);

impl Person {
    fn roll(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11) as f64 / (1u64 << 53) as f64
    }
}

async fn simulate(options: &Options, mut args: Vec<String>) -> CliResult {
    let days: i64 =
        take_value(&mut args, "--days")?.ok_or("simulate --days N")?.parse().map_err(|_| "--days needs a number")?;
    let seed: u64 =
        take_value(&mut args, "--seed")?.map_or(Ok(1), |s| s.parse().map_err(|_| "--seed needs a number"))?;
    let real_model = take_flag(&mut args, "--real-model");
    if let Some(extra) = args.first() {
        return Err(format!("simulate doesn't take {extra:?}"));
    }

    let embedder: Arc<dyn Embedder> = if real_model {
        Arc::new(
            CandleEmbedder::load(&options.model_dir)
                .map_err(|error| format!("{error} (run scripts/fetch-model.sh)"))?,
        )
    } else {
        Arc::new(HashingEmbedder)
    };
    let dir = std::env::temp_dir().join(format!("autopaper-simulate-{}-{seed}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let clock = Arc::new(FixedClock::at(SIMULATION_START));
    let config = EngineConfig {
        data_dir: dir.to_string_lossy().into_owned(),
        model_dir: String::new(),
        locale: "en-US".into(),
        client: "cli-simulate".into(),
    };
    let secrets: Arc<dyn SecretStore> = Arc::new(EnvSecrets::default());
    let deps =
        Deps { clock: clock.clone(), http: Arc::new(StubHttp::new()), embedder: embedder.clone(), rng_seed: seed };
    let engine = Engine::open_with(config, secrets, deps).map_err(err)?;
    // Small paintings keep the run fast; sizing works the same at any size.
    engine.set_display_hint(384, 216).map_err(err)?;
    let musts = ["lake"];
    let maybes = ["mist", "lanterns", "ruins", "autumn", "boats", "mountains"];
    let avoids = ["people"];
    for (texts, weight) in
        [(&musts[..], KeywordWeight::Must), (&maybes[..], KeywordWeight::Maybe), (&avoids[..], KeywordWeight::Avoid)]
    {
        for text in texts {
            engine.add_keyword(text.to_string(), weight).map_err(err)?;
        }
    }
    let settings = engine.settings().map_err(err)?;
    say!(
        "Simulating {days} days from {} (seed {seed}): Demo providers, {} embeddings, cadence {:?}, quiet period {} days, \
echoes {:?}, surprise {:.2}",
        date(SIMULATION_START),
        embedder.model_id(),
        settings.cadence,
        settings.quiet_period.days(),
        settings.echoes,
        settings.surprise
    );
    say!("Keywords: Must {} · Maybe {} · Avoid {}", musts.join(", "), maybes.join(", "), avoids.join(", "));

    let started = Instant::now();
    let mut person = Person(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let (mut scheduled, mut replaced, mut revisits, mut likes, mut dislikes) = (0u32, 0u32, 0u32, 0u32, 0u32);
    for _ in 0..days {
        clock.advance_days(1.0);
        let shown = match engine.run_if_due(None).await {
            Ok(Some(shown)) => shown,
            Ok(None) => continue,
            Err(error) => return Err(format!("day {}: {}", (now(&clock) - SIMULATION_START) / DAY, err(error))),
        };
        if shown.revisit.is_some() {
            revisits += 1;
        } else {
            scheduled += 1;
        }
        engine.mark_shown(shown.generation.id.clone()).map_err(err)?;
        let roll = person.roll();
        if roll < 0.2 {
            engine.rate(shown.generation.id, Rating::Liked).map_err(err)?;
            likes += 1;
        } else if roll < 0.27 {
            dislikes += 1;
            if engine.rate(shown.generation.id, Rating::Disliked).map_err(err)? {
                let replacement = engine.generate(Trigger::DislikeReplace, None).await.map_err(err)?;
                engine.mark_shown(replacement.id).map_err(err)?;
                replaced += 1;
            }
        }
    }
    let elapsed = started.elapsed();

    let store = Store::open(&dir.join("autopaper.sqlite3")).map_err(err)?;
    let memory = store.memory().map_err(err)?;
    let stats = engine.stats();
    say!();
    say!(
        "Generations: {} ({scheduled} scheduled, {replaced} replacing dislikes; {revisits} revisits)",
        memory.len()
    );
    say!("Ratings: {likes} likes, {dislikes} dislikes");

    let echoes = engine.history(HistoryFilter::Echoes, 100_000, 0).map_err(err)?;
    say!("Echoes: {}", echoes.len());
    for echo in echoes.iter().rev().take(12) {
        let original = echo.echo_of.clone().and_then(|id| engine.generation(id).ok());
        let (title, age) = original
            .map_or(("(forgotten)".to_string(), 0), |o| (o.concept.title, (echo.created_at - o.created_at) / DAY));
        say!("  {} “{}” ← “{title}” ({age} days earlier)", date(echo.created_at), echo.concept.title);
    }
    if echoes.len() > 12 {
        say!("  … and {} more", echoes.len() - 12);
    }

    let calibration = Calibration::for_model(embedder.model_id());
    let quiet = settings.quiet_period.days() * DAY;
    let (mut closest, mut pair) = (f32::MIN, None);
    let mut echo_similarity: Vec<f32> = Vec::new();
    let by_id: HashMap<&str, usize> = memory.iter().enumerate().map(|(i, row)| (row.id.as_str(), i)).collect();
    for (i, a) in memory.iter().enumerate() {
        for b in &memory[..i] {
            if (a.created_at - b.created_at).abs() <= quiet {
                let similarity = cosine(&a.embedding, &b.embedding);
                if similarity > closest {
                    closest = similarity;
                    pair = Some((b.title.clone(), a.title.clone()));
                }
            }
        }
        if let Some(original) = a.echo_of.as_deref().and_then(|id| by_id.get(id)) {
            echo_similarity.push(cosine(&a.embedding, &memory[*original].embedding));
        }
    }
    say!(
        "Novelty: {} compose calls; {} candidates passed over as too similar; {} retries for novelty, {} for \
invalid candidates, {} not made (no closer); {} least-similar fallbacks; keywords narrow at the end: {}",
        stats.compose_calls,
        stats.too_similar_candidates,
        stats.novelty_retries,
        stats.invalid_retries,
        stats.retries_stopped,
        stats.least_similar_fallbacks,
        if engine.keywords_are_narrow().map_err(err)? { "yes" } else { "no" }
    );
    if let Some((first, second)) = pair {
        say!(
            "Most similar pair within the quiet period: {closest:.3} (threshold {:.2}): “{first}” / “{second}”",
            calibration.threshold
        );
    }
    if !echo_similarity.is_empty() {
        let min = echo_similarity.iter().copied().fold(f32::INFINITY, f32::min);
        let max = echo_similarity.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let inside = echo_similarity.iter().filter(|s| calibration.in_echo_band(**s)).count();
        say!(
            "Echo similarity to originals: {min:.3}–{max:.3}, {inside} of {} inside the band {:?}",
            echo_similarity.len(),
            calibration.echo_band
        );
    }
    let taste = engine.taste_summary().map_err(err)?;
    say!("Taste learned ({} ratings): likes {}", taste.ratings, list_or_none(&taste.liked));
    say!("  dislikes {}", list_or_none(&taste.disliked));
    let usage = engine.storage_usage().map_err(err)?;
    let spend = engine.spend_summary().map_err(err)?;
    say!(
        "Storage: {} images on disk, {:.1} MB · spend this month {} · simulated in {:.1}s",
        usage.images_on_disk,
        usage.image_bytes as f64 / 1_048_576.0,
        money(spend.spent_microusd),
        elapsed.as_secs_f32()
    );
    drop(store);
    drop(engine);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

fn now(clock: &FixedClock) -> i64 {
    autopaper_core::ports::Clock::now(clock)
}

// ── Formatting ──────────────────────────────────────────────────────────────────────────────

fn short(id: &str) -> &str {
    // UUIDv7s share their leading time bits; the last block tells them apart.
    id.rsplit('-').next().map_or(id, |tail| &tail[..tail.len().min(8)])
}

fn money(microusd: u64) -> String {
    format!("${:.4}", microusd as f64 / 1_000_000.0)
}

fn list_or_none(items: &[String]) -> String {
    if items.is_empty() { "—".to_string() } else { items.join(", ") }
}

/// "YYYY-MM-DD" (UTC), Howard Hinnant's civil-from-days.
fn date(unix: i64) -> String {
    let z = unix.div_euclid(DAY) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

fn datetime(unix: i64) -> String {
    let seconds = unix.rem_euclid(DAY);
    format!("{} {:02}:{:02} UTC", date(unix), seconds / 3600, seconds % 3600 / 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_env_files() {
        let text = "# keys\nOPENAI_API_KEY=sk-abc\nexport GEMINI_API_KEY=\"AIza x\"\nEMPTY=\nOTHER='q' \n  # x=y\nBARE=v # note";
        assert_eq!(
            parse_env(text),
            [
                ("OPENAI_API_KEY".to_string(), "sk-abc".to_string()),
                ("GEMINI_API_KEY".to_string(), "AIza x".to_string()),
                ("OTHER".to_string(), "q".to_string()),
                ("BARE".to_string(), "v".to_string()),
            ]
        );
    }

    #[test]
    fn reads_display_sizes() {
        assert_eq!(parse_display("4112x2658"), Ok((4112, 2658)));
        assert_eq!(parse_display("3840×2160"), Ok((3840, 2160)));
        assert_eq!(parse_display(" 2560 X 1600 "), Ok((2560, 1600)));
        for bad in ["4112", "x2658", "4112x", "big", "-1x5"] {
            assert!(parse_display(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn printed_text_cannot_drive_the_terminal() {
        assert_eq!(printable("Lighthouse\u{1b}]52;c;QUJD\u{7} at dusk\r"), "Lighthouse ]52;c;QUJD  at dusk ");
        assert_eq!(printable("{\n  \"a\": 1\n}"), "{\n  \"a\": 1\n}", "line breaks stay");
    }

    #[test]
    fn formats_dates_and_ids() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(SIMULATION_START), "2026-01-01");
        assert_eq!(date(1_709_208_000), "2024-02-29");
        assert_eq!(datetime(SIMULATION_START + 3_723), "2026-01-01 01:02 UTC");
        assert_eq!(short("0199b1c2-1234-7abc-8def-0123456789ab"), "01234567");
        assert_eq!(money(100_080), "$0.1001");
    }

    #[test]
    fn lists_a_moods_keywords_in_short() {
        let keyword = |text: &str, weight| Keyword { id: text.into(), text: text.into(), weight, position: 0, created_at: 0 };
        let keywords = [keyword("rain", KeywordWeight::Must), keyword("beach", KeywordWeight::Maybe), keyword("people", KeywordWeight::Avoid)];
        assert_eq!(keywords_in_short(&keywords), "rain · beach · no people");
        assert_eq!(keywords_in_short(&[]), "no keywords yet");
    }

    #[test]
    fn the_simulated_person_is_reproducible() {
        let (mut a, mut b) = (Person(7), Person(7));
        let rolls: Vec<f64> = (0..100).map(|_| a.roll()).collect();
        assert!(rolls.iter().all(|r| (0.0..1.0).contains(r)));
        assert_eq!(rolls, (0..100).map(|_| b.roll()).collect::<Vec<_>>());
    }
}
