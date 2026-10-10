//! The engine end to end, with fakes only: Demo providers (or OpenAI over a scripted `StubHttp`), a
//! `FixedClock` moved by hand across simulated years, the `HashingEmbedder`, and a temporary data
//! directory. No network, no model files, no keys beyond test strings.

use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::task::Poll;

use autopaper_core::embed::{HashingEmbedder, cosine};
use autopaper_core::engine::{DEFAULT_DISPLAY, Deps, Engine, EngineConfig, RevisitReason};
use autopaper_core::error::{AutoPaperError, InvalidInputReason, ProviderUnavailableReason};
use autopaper_core::model::*;
use autopaper_core::novelty::Calibration;
use autopaper_core::ports::{
    Clock, Embedder, HttpClient, HttpRequest, HttpResponse, ProgressDetailObserver, ProgressObserver, SecretStore, SystemModel,
};
use autopaper_core::store::{MemoryRow, Store};
use autopaper_core::testing::{FixedClock, SocketStep, StubHttp, StubSecrets};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::json;
use tempfile::TempDir;

/// 2026-01-01T00:00:00Z.
const START: i64 = 1_767_225_600;
const DAY: i64 = 86_400;
/// Small paintings keep long runs fast; the engine's sizing is the same at any size.
const DISPLAY: (u32, u32) = (128, 72);

struct Harness {
    engine: Arc<Engine>,
    clock: Arc<FixedClock>,
    http: Arc<StubHttp>,
    dir: TempDir,
}

impl Harness {
    fn new() -> Self {
        Self::with(Arc::new(HashingEmbedder), 7)
    }

    fn with(embedder: Arc<dyn Embedder>, seed: u64) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let clock = Arc::new(FixedClock::at(START));
        let http = Arc::new(StubHttp::new());
        let engine = open(dir.path(), clock.clone(), http.clone(), embedder, seed);
        Self { engine, clock, http, dir }
    }

    /// The same data directory opened again (another process, or the next launch).
    fn reopen(&mut self, embedder: Arc<dyn Embedder>) {
        self.engine = open(self.dir.path(), self.clock.clone(), self.http.clone(), embedder, 11);
    }

    fn store(&self) -> Store {
        Store::open(&self.dir.path().join("autopaper.sqlite3")).expect("open the store")
    }

    fn memory(&self) -> Vec<MemoryRow> {
        self.store().memory().expect("memory")
    }

    fn now(&self) -> i64 {
        self.clock.now()
    }

    fn keywords(&self, musts: &[&str], maybes: &[&str], avoids: &[&str]) {
        for (texts, weight) in
            [(musts, KeywordWeight::Must), (maybes, KeywordWeight::Maybe), (avoids, KeywordWeight::Avoid)]
        {
            for text in texts {
                self.engine.add_keyword(text.to_string(), weight).expect("add keyword");
            }
        }
    }

    fn update(&self, change: impl FnOnce(&mut Settings)) {
        let mut settings = self.engine.settings().expect("settings");
        change(&mut settings);
        self.engine.update_settings(settings).expect("update settings");
    }

    async fn generate(&self) -> Generation {
        self.engine.generate(Trigger::Manual, None).await.expect("generate")
    }
}

fn open(
    dir: &Path,
    clock: Arc<FixedClock>,
    http: Arc<dyn HttpClient>,
    embedder: Arc<dyn Embedder>,
    seed: u64,
) -> Arc<Engine> {
    let config = EngineConfig {
        data_dir: dir.to_string_lossy().into_owned(),
        model_dir: String::new(),
        locale: "en-US".into(),
        client: "test".into(),
    };
    let secrets: Arc<dyn SecretStore> =
        Arc::new(StubSecrets::with(&[("openai.api_key", "sk-test-0123456789abcdefghij")]));
    let engine = Engine::open_with(config, secrets, Deps { clock, http, embedder, rng_seed: seed }).expect("open");
    engine.set_display_hint(DISPLAY.0, DISPLAY.1).expect("display hint");
    engine
}

/// Successful availability fixtures; generation/error responses are still scripted by each test.
fn healthy_services(http: &StubHttp) {
    http.always("/v1/models", 200, br#"{"data":[]}"#.to_vec());
    http.always("/api/tags", 200, br#"{"models":[{"name":"m"}]}"#.to_vec());
    http.always("/object_info", 200, br#"{}"#.to_vec());
}

fn work_requests(http: &StubHttp) -> Vec<HttpRequest> {
    http.requests().into_iter().filter(|request| {
        !request.url.ends_with("/v1/models") && !request.url.ends_with("/api/tags") && !request.url.ends_with("/object_info")
    }).collect()
}

fn work_json(http: &StubHttp, index: usize) -> serde_json::Value {
    serde_json::from_slice(work_requests(http)[index].body.as_deref().unwrap()).unwrap()
}

fn exists(path: &Option<String>) -> bool {
    path.as_deref().is_some_and(|path| Path::new(path).is_file())
}

#[derive(Default)]
struct Stages(Mutex<Vec<ProgressStage>>);

impl ProgressObserver for Stages {
    fn on_progress(&self, stage: ProgressStage) {
        self.0.lock().unwrap().push(stage);
    }
}

// ── Making, showing, describing ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn keywords_to_a_stored_wallpaper_that_becomes_current() {
    let h = Harness::new();
    h.keywords(&["lighthouse"], &["fog", "cliffs"], &["people"]);
    let keywords = h.engine.keywords().unwrap();
    assert_eq!(keywords.iter().map(|k| k.text.as_str()).collect::<Vec<_>>(), ["lighthouse", "fog", "cliffs", "people"]);
    // A case-insensitive duplicate updates the weight.
    let again = h.engine.add_keyword("FOG".into(), KeywordWeight::Must).unwrap();
    assert_eq!((again.text.as_str(), again.weight), ("fog", KeywordWeight::Must));

    let stages = Arc::new(Stages::default());
    let observer: Arc<dyn ProgressObserver> = stages.clone();
    let generation = h.engine.generate(Trigger::Manual, Some(observer)).await.expect("generate");
    assert_eq!(
        *stages.0.lock().unwrap(),
        [
            ProgressStage::CheckingServices,
            ProgressStage::Composing,
            ProgressStage::CheckingMemory,
            ProgressStage::Generating,
            ProgressStage::Downloading,
            ProgressStage::Rendering,
            ProgressStage::Done
        ]
    );
    assert_eq!(generation.status, GenerationStatus::Ok);
    assert_eq!(generation.trigger, Trigger::Manual);
    assert_eq!((generation.text_provider, generation.image_provider), (ProviderKind::Demo, ProviderKind::Demo));
    assert!(autopaper_core::text::mentions(&generation.concept.prompt, "lighthouse"));
    assert!(autopaper_core::text::mentions(&generation.concept.prompt, "fog"));
    assert!(!autopaper_core::text::mentions(&generation.concept.prompt, "people"));
    assert!(exists(&generation.image_path) && exists(&generation.thumb_path));
    // Either separator: Windows paths use backslashes.
    assert!(generation.image_path.as_deref().unwrap().replace('\\', "/").contains("/images/2026/"));
    assert_eq!((generation.width, generation.height), DISPLAY, "free sizes stop at the display's pixels");
    assert_eq!(generation.keywords.len(), 4);
    assert_eq!(generation.cost_microusd, 0);

    assert_eq!(h.engine.current().unwrap(), None, "nothing is current until the host shows it");
    h.engine.mark_shown(generation.id.clone()).unwrap();
    let current = h.engine.current().unwrap().expect("current");
    assert_eq!(current.id, generation.id);
    assert_eq!((current.shown_count, current.last_shown_at), (1, Some(START)));

    let display = DisplayTarget { id: "main".into(), width: 160, height: 100 };
    let render = h.engine.render_for_display(generation.id.clone(), display.clone()).unwrap();
    assert!(render.replace('\\', "/").ends_with(&format!("renders/{}-160x100.jpg", generation.id)), "{render}");
    let (width, height) = image::image_dimensions(&render).unwrap();
    assert_eq!((width, height), (160, 100));
    assert_eq!(h.engine.render_for_display(generation.id.clone(), display).unwrap(), render, "cached");
    let too_big = DisplayTarget { id: "x".into(), width: 20_000, height: 100 };
    assert!(matches!(
        h.engine.render_for_display(generation.id.clone(), too_big),
        Err(AutoPaperError::InvalidInput { reason: InvalidInputReason::DisplaySizeInvalid, .. })
    ));
    assert!(matches!(
        h.engine.set_display_hint(0, 1080),
        Err(AutoPaperError::InvalidInput { reason: InvalidInputReason::DisplaySizeInvalid, .. })
    ));
    assert!(matches!(
        h.engine.add_keyword("x".repeat(41), KeywordWeight::Must),
        Err(AutoPaperError::InvalidInput { reason: InvalidInputReason::KeywordTooLong, .. })
    ));
    let unknown = DisplayTarget { id: "x".into(), width: 10, height: 10 };
    assert!(matches!(h.engine.render_for_display("nope".into(), unknown), Err(AutoPaperError::NotFound)));

    let description = h.engine.describe(generation.id.clone()).unwrap();
    assert!(description.starts_with(generation.concept.title.trim()), "{description}");
    assert!(description.contains(generation.concept.summary.trim()), "{description}");
    h.engine.rate(generation.id.clone(), Rating::Liked).unwrap();
    assert_eq!(h.engine.describe(generation.id.clone()).unwrap(), description, "hosts say the rating themselves");
    h.engine.rate(generation.id.clone(), Rating::Unrated).unwrap();
    assert_eq!(h.engine.history(HistoryFilter::All, 10, 0).unwrap(), std::slice::from_ref(&current));
    assert_eq!(h.engine.generation(generation.id.clone()).unwrap(), current);

    let usage = h.engine.storage_usage().unwrap();
    assert_eq!((usage.generations, usage.images_on_disk), (1, 1));
    assert!(usage.image_bytes > 0);
    let spend = h.engine.spend_summary().unwrap();
    assert_eq!(
        (spend.month.as_str(), spend.spent_microusd, spend.images, spend.per_image_microusd),
        ("2026-01", 0, 1, 0)
    );
}

#[tokio::test]
async fn the_default_display_is_4k_and_hosted_prices_follow_it() {
    let h = Harness::new();
    h.engine.set_display_hint(DEFAULT_DISPLAY.0, DEFAULT_DISPLAY.1).unwrap();
    h.update(|s| {
        s.text_provider.kind = ProviderKind::OpenAi;
        s.image_provider.kind = ProviderKind::OpenAi;
        s.cadence = Cadence::Daily;
    });
    let spend = h.engine.spend_summary().unwrap();
    // gpt-image-2.5-flare at 3840×2160, high quality: 100,080 µ$; plus a typical gpt-6-luna compose call.
    assert!(spend.per_image_microusd > 100_080 && spend.per_image_microusd < 105_000, "{spend:?}");
    assert_eq!(spend.monthly_estimate_microusd, spend.per_image_microusd * 30);
    assert!(h.http.requests().is_empty(), "estimates make no calls");
}

// ── Learning ────────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ratings_update_taste_and_say_when_to_replace() {
    let h = Harness::new();
    h.keywords(&["lighthouse"], &[], &[]);
    let first = h.generate().await;
    h.engine.mark_shown(first.id.clone()).unwrap();
    let second = h.generate().await;

    assert!(!h.engine.rate(second.id.clone(), Rating::Disliked).unwrap(), "not the one showing");
    assert!(h.engine.rate(first.id.clone(), Rating::Disliked).unwrap(), "disliked while showing: replace");
    assert!(!h.engine.rate(first.id.clone(), Rating::Disliked).unwrap(), "the same rating again changes nothing");
    h.update(|s| s.replace_disliked = false);
    h.engine.rate(first.id.clone(), Rating::Unrated).unwrap();
    assert!(!h.engine.rate(first.id.clone(), Rating::Disliked).unwrap(), "the setting is off");
    assert!(matches!(h.engine.rate("nope".into(), Rating::Liked), Err(AutoPaperError::NotFound)));

    let summary = h.engine.taste_summary().unwrap();
    assert_eq!(summary.ratings, 2);
    assert!(summary.disliked.iter().any(|feature| feature == "lighthouse"), "{summary:?}");

    for _ in 0..3 {
        let liked = h.generate().await;
        h.engine.rate(liked.id, Rating::Liked).unwrap();
    }
    h.engine.rate(first.id.clone(), Rating::Liked).unwrap();
    h.engine.rate(second.id.clone(), Rating::Liked).unwrap();
    let summary = h.engine.taste_summary().unwrap();
    assert_eq!(summary.ratings, 5);
    assert!(summary.liked.iter().any(|feature| feature == "lighthouse"), "{summary:?}");
    assert!(summary.disliked.is_empty(), "{summary:?}");
    // In the words the wallpapers used, not stem keys ("lighthous", "warm ivor").
    let phrases: Vec<String> = h
        .engine
        .history(HistoryFilter::Liked, 10, 0)
        .unwrap()
        .iter()
        .flat_map(|g| autopaper_core::taste::labelled_features(&g.concept).into_iter().map(|(_, phrase)| phrase))
        .collect();
    assert!(summary.liked.iter().all(|feature| phrases.contains(feature)), "{summary:?} vs {phrases:?}");
    assert_eq!(h.engine.history(HistoryFilter::Liked, 10, 0).unwrap().len(), 5);

    h.engine.reset_taste().unwrap();
    let summary = h.engine.taste_summary().unwrap();
    assert!(summary.liked.is_empty() && summary.disliked.is_empty());
    assert_eq!(summary.ratings, 5, "ratings stay on the generations");
}

// ── Novelty ─────────────────────────────────────────────────────────────────────────────────

fn candidate(
    title: &str,
    summary: &str,
    setting: &str,
    elements: &[&str],
    palette: &[&str],
    prompt: &str,
) -> serde_json::Value {
    json!({
        "setting": setting, "subject": elements[0], "elements": elements, "time_of_day": "dusk",
        "weather": "clear", "season": "autumn", "mood": ["calm"], "palette": palette,
        "style": "oil painting", "composition": "low horizon, open sky on the left",
        "keywords_used": ["lighthouse"], "wildcards": [], "prompt": prompt, "title": title, "summary": summary
    })
}

/// A Responses API answer whose output text is `{"candidates": candidates}`.
fn responses(candidates: &[serde_json::Value]) -> Vec<u8> {
    responses_text("completed", &json!({ "candidates": candidates }).to_string())
}

/// A Responses API answer with this status and output text (and the usage every answer here reports).
fn responses_text(status: &str, text: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "id": "resp_1", "object": "response", "status": status, "model": "gpt-6-luna-2026-09-22",
        "incomplete_details": if status == "incomplete" { json!({ "reason": "max_output_tokens" }) } else { json!(null) },
        "output": [{ "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": text }] }],
        "usage": { "input_tokens": 1800, "output_tokens": 1500 }
    }))
    .unwrap()
}

#[tokio::test]
async fn a_repeat_is_retried_naming_what_it_was_too_close_to() {
    let h = Harness::new();
    healthy_services(&h.http);
    h.keywords(&["lighthouse"], &[], &[]);
    h.update(|s| s.text_provider.kind = ProviderKind::OpenAi);
    let harbour = candidate(
        "Harbour lighthouse at dusk",
        "A white lighthouse on a stone pier at dusk, gulls over calm water.",
        "a fishing harbour",
        &["white lighthouse", "stone pier", "gulls", "moored boats"],
        &["amber", "slate blue", "white"],
        "An oil painting of a white lighthouse on a stone pier in a fishing harbour at dusk, gulls over calm water.",
    );
    let desert = candidate(
        "Lighthouse in the dunes",
        "A rusted lighthouse half buried in red sand dunes under a violet sky.",
        "a desert of red dunes",
        &["rusted lighthouse", "sand ripples", "dry grass", "distant mesa"],
        &["rust", "violet", "ochre"],
        "An oil painting of a rusted lighthouse half buried in red sand dunes beneath a violet evening sky.",
    );
    h.http.once("/v1/responses", 200, responses(std::slice::from_ref(&harbour)));
    h.http.once("/v1/responses", 200, responses(&[harbour.clone(), harbour.clone()]));
    h.http.once("/v1/responses", 200, responses(&[desert]));

    let first = h.generate().await;
    assert_eq!(first.concept.title, "Harbour lighthouse at dusk");
    assert_eq!((first.text_provider, first.text_model.as_str()), (ProviderKind::OpenAi, "gpt-6-luna-2026-09-22"));
    assert!(first.cost_microusd > 0, "text calls are priced from reported usage");

    let second = h.generate().await;
    assert_eq!(second.concept.title, "Lighthouse in the dunes");
    let stats = h.engine.stats();
    assert_eq!((stats.compose_calls, stats.novelty_retries, stats.least_similar_fallbacks), (3, 1, 0));

    let requests = work_requests(&h.http);
    assert_eq!(requests.len(), 3);
    let retry = String::from_utf8(requests[2].body.clone().unwrap()).unwrap();
    assert!(retry.contains("too close to these past wallpapers"), "the retry says why");
    assert!(retry.contains("A white lighthouse on a stone pier at dusk, gulls over calm water."), "and names it");
    let first_ask = String::from_utf8(requests[1].body.clone().unwrap()).unwrap();
    assert!(!first_ask.contains("too close to these past wallpapers"));

    let spend = h.engine.spend_summary().unwrap();
    assert_eq!(spend.spent_microusd, first.cost_microusd + second.cost_microusd);
    assert_eq!(second.cost_microusd, 2 * first.cost_microusd, "both calls for the second wallpaper are counted");
}

#[tokio::test]
async fn when_nothing_novel_comes_back_the_least_similar_valid_candidate_wins() {
    let h = Harness::new();
    healthy_services(&h.http);
    h.keywords(&["lighthouse"], &[], &["people"]);
    h.update(|s| s.text_provider.kind = ProviderKind::OpenAi);
    let harbour = candidate(
        "Harbour lighthouse at dusk",
        "A white lighthouse on a stone pier at dusk, gulls over calm water.",
        "a fishing harbour",
        &["white lighthouse", "stone pier", "gulls", "moored boats"],
        &["amber", "slate blue", "white"],
        "An oil painting of a white lighthouse on a stone pier in a fishing harbour at dusk, gulls over calm water.",
    );
    let crowded = candidate(
        "Crowded pier",
        "People crowd a pier below a lighthouse.",
        "a pier",
        &["people", "lighthouse"],
        &["red"],
        "People on a pier below a lighthouse.",
    );
    h.http.once("/v1/responses", 200, responses(std::slice::from_ref(&harbour)));
    h.http.always("/v1/responses", 200, responses(&[harbour.clone(), crowded]));
    let first = h.generate().await;
    let second = h.generate().await;
    assert_eq!(second.concept.title, "Harbour lighthouse at dusk", "the only valid one, though not novel");
    // The retry came back no closer to a new idea (the same repeat), so there's no second retry.
    let stats = h.engine.stats();
    assert_eq!(
        (stats.compose_calls, stats.novelty_retries, stats.retries_stopped, stats.least_similar_fallbacks),
        (3, 1, 1, 1)
    );
    assert_eq!(work_requests(&h.http).len(), 3);
    let least_similar = |id: &str| h.store().generation(id).unwrap().unwrap().least_similar;
    assert!(least_similar(&second.id), "the fallback is recorded on the generation");
    assert!(!least_similar(&first.id));
}

/// The harbour candidate reworded: still too similar to it (hashing threshold 0.70), but less each time.
fn harbour_reworded(times: usize) -> serde_json::Value {
    match times {
        1 => candidate(
            "Harbour lighthouse at dusk",
            "A white lighthouse on a stone pier at dusk, gulls over still water.",
            "a fishing harbour",
            &["white lighthouse", "stone pier", "gulls", "fishing nets"],
            &["amber", "slate blue", "white"],
            "An oil painting of a white lighthouse on a stone pier at dusk, gulls over still water, fishing nets.",
        ),
        _ => candidate(
            "Harbour lighthouse in the evening",
            "A white lighthouse on a stone jetty in the evening, terns over still water.",
            "a fishing harbour",
            &["white lighthouse", "stone jetty", "terns", "fishing nets"],
            &["amber", "slate blue", "cream"],
            "An oil painting of a white lighthouse on a stone jetty in the evening, terns over still water.",
        ),
    }
}

#[tokio::test]
async fn retries_go_on_while_they_come_closer_to_a_new_idea() {
    let h = Harness::new();
    healthy_services(&h.http);
    h.keywords(&["lighthouse"], &[], &[]);
    h.update(|s| s.text_provider.kind = ProviderKind::OpenAi);
    h.http.once("/v1/responses", 200, responses(&[harbour()]));
    h.http.once("/v1/responses", 200, responses(&[harbour()]));
    h.http.once("/v1/responses", 200, responses(&[harbour_reworded(1)]));
    h.http.once("/v1/responses", 200, responses(&[harbour_reworded(2)]));
    h.generate().await;
    let second = h.generate().await;

    // Each retry lowered the penalty (1.0, then about 0.96, then 0.72), so both were made; none got under the
    // threshold, so the least similar one is taken.
    let threshold = Calibration::for_model(HashingEmbedder::MODEL_ID).threshold;
    let memory = h.memory();
    let similarity = cosine(&memory[0].embedding, &memory[1].embedding);
    assert!(similarity >= threshold && similarity < 0.8, "{similarity}");
    assert_eq!(second.concept.title, "Harbour lighthouse in the evening");
    let stats = h.engine.stats();
    assert_eq!(
        (stats.compose_calls, stats.novelty_retries, stats.retries_stopped, stats.least_similar_fallbacks),
        (4, 2, 0, 1)
    );
    assert!(h.store().generation(&second.id).unwrap().unwrap().least_similar);
}

#[tokio::test]
async fn keywords_are_narrow_when_most_recent_wallpapers_fell_back() {
    let h = Harness::new();
    healthy_services(&h.http);
    h.keywords(&["lighthouse"], &[], &[]);
    h.update(|s| s.text_provider.kind = ProviderKind::OpenAi);
    assert!(!h.engine.keywords_are_narrow().unwrap(), "nothing made yet");
    // One idea, over and over: after the first, every wallpaper is the least similar repeat (two calls each).
    for _ in 0..7 {
        h.http.once("/v1/responses", 200, responses(&[harbour()]));
    }
    for (made, narrow) in [(1, false), (2, false), (3, false), (4, true)] {
        h.generate().await;
        h.clock.advance_days(1.0);
        assert_eq!(h.engine.keywords_are_narrow().unwrap(), narrow, "after {made}");
    }
    // New ideas come back once the keywords have room (a Maybe added, say): the note goes once fewer than
    // three of the last five fell back.
    let mut forest = candidate(
        "Lighthouse among snowy pines",
        "A stone lighthouse stands in a snowbound pine forest at dawn, its lamp glowing through falling snow.",
        "a frozen pine forest",
        &["stone lighthouse", "snow-laden pines", "frozen stream", "fox tracks"],
        &["ice blue", "pine green", "gold"],
        "An oil painting of a stone lighthouse in a snowbound pine forest at dawn, its lamp glowing through snow.",
    );
    forest["time_of_day"] = json!("dawn");
    forest["weather"] = json!("snowfall");
    forest["season"] = json!("winter");
    forest["mood"] = json!(["hushed"]);
    let mut city = candidate(
        "Lighthouse above a neon city",
        "A steel lighthouse rises over rain-slick city rooftops at midnight, its beam crossing neon signs.",
        "a dense city of rooftops",
        &["steel lighthouse", "wet rooftops", "neon signs", "water towers"],
        &["magenta", "teal", "black"],
        "An oil painting of a steel lighthouse over rain-slick city rooftops at midnight, its beam crossing neon signs.",
    );
    city["time_of_day"] = json!("midnight");
    city["weather"] = json!("rain");
    city["season"] = json!("spring");
    city["mood"] = json!(["electric"]);
    let desert = candidate(
        "Lighthouse in the dunes",
        "A rusted lighthouse half buried in red sand dunes under a violet sky.",
        "a desert of red dunes",
        &["rusted lighthouse", "sand ripples", "dry grass", "distant mesa"],
        &["rust", "violet", "ochre"],
        "An oil painting of a rusted lighthouse half buried in red sand dunes beneath a violet evening sky.",
    );
    for (idea, narrow) in [(desert, true), (forest, true), (city, false)] {
        h.http.once("/v1/responses", 200, responses(&[idea]));
        let made = h.generate().await;
        assert!(!h.store().generation(&made.id).unwrap().unwrap().least_similar, "{}", made.concept.title);
        h.clock.advance_days(1.0);
        assert_eq!(h.engine.keywords_are_narrow().unwrap(), narrow, "after {}", made.concept.title);
    }
    assert_eq!(work_requests(&h.http).len(), 1 + 3 * 2 + 3, "repeats stopped after one retry each");
}

// ── Long runs: novelty within the quiet period, echoes after it ────────────────────────────

struct RunSummary {
    shown: u32,
    replaced: u32,
}

/// The host's loop: every `step_days` the timer fires; a new wallpaper is shown, then liked (20%) or
/// disliked (5%) at random, and a disliked one that should be replaced is (as hosts do).
async fn simulate(h: &Harness, steps: i64, step_days: f64, seed: u64) -> RunSummary {
    let mut rng = StdRng::seed_from_u64(seed);
    let mut summary = RunSummary { shown: 0, replaced: 0 };
    for _ in 0..steps {
        h.clock.advance_days(step_days);
        let Some(shown) = h.engine.run_if_due(None).await.expect("run_if_due") else { continue };
        assert_eq!(shown.revisit, None);
        h.engine.mark_shown(shown.generation.id.clone()).unwrap();
        summary.shown += 1;
        let roll: f64 = rng.random();
        if roll < 0.2 {
            h.engine.rate(shown.generation.id, Rating::Liked).unwrap();
        } else if roll < 0.25 && h.engine.rate(shown.generation.id, Rating::Disliked).unwrap() {
            let replacement = h.engine.generate(Trigger::DislikeReplace, None).await.expect("replace");
            h.engine.mark_shown(replacement.id).unwrap();
            summary.replaced += 1;
        }
    }
    summary
}

/// Highest cosine between two remembered wallpapers made within `quiet_days` of each other.
fn closest_within(memory: &[MemoryRow], quiet_days: i64) -> f32 {
    let mut closest = 0.0f32;
    for (i, a) in memory.iter().enumerate() {
        for b in &memory[..i] {
            if (a.created_at - b.created_at).abs() <= quiet_days * DAY {
                closest = closest.max(cosine(&a.embedding, &b.embedding));
            }
        }
    }
    closest
}

#[tokio::test]
async fn every_wallpaper_within_the_quiet_period_stays_novel() {
    // Daily for eight months with a six-month quiet period: up to 182 earlier wallpapers to avoid each time,
    // and four Musts that every wallpaper shares, so memory has to push back.
    let h = Harness::new();
    h.keywords(&["lake", "mist", "boats", "pines"], &["reeds", "stars"], &["people"]);
    h.update(|s| {
        s.cadence = Cadence::Daily;
        s.quiet_period = QuietPeriod::SixMonths;
        s.echoes = EchoFrequency::Off;
    });
    let run = simulate(&h, 240, 1.0, 5).await;
    assert_eq!(run.shown, 240, "a wallpaper every day");
    let memory = h.memory();
    assert_eq!(memory.len() as u32, run.shown + run.replaced);
    assert!(memory.iter().all(|row| row.embedding_model == HashingEmbedder::MODEL_ID && row.echo_of.is_none()));

    let threshold = Calibration::for_model(HashingEmbedder::MODEL_ID).threshold;
    let closest = closest_within(&memory, QuietPeriod::SixMonths.days());
    assert!(closest < threshold, "two wallpapers within the quiet period at {closest:.3}");
    let stats = h.engine.stats();
    assert!(stats.too_similar_candidates > 0, "memory never pushed back: {stats:?}");
    assert_eq!(stats.least_similar_fallbacks, 0, "{stats:?}");
    assert!(h.engine.taste_summary().unwrap().ratings > 30);
}

#[tokio::test]
async fn over_three_years_echoes_come_only_after_the_quiet_period_and_stay_in_band() {
    let h = Harness::new();
    h.keywords(&["lighthouse"], &["fog", "cliffs", "harbour", "storm"], &[]);
    h.update(|s| {
        s.cadence = Cadence::Weekly;
        s.quiet_period = QuietPeriod::OneMonth;
        s.echoes = EchoFrequency::Often;
        s.surprise = 0.6;
    });
    let weeks = (3 * 365 + 30) / 7;
    let run = simulate(&h, weeks, 7.0, 99).await;
    assert_eq!(run.shown as i64, weeks, "a wallpaper every week");

    let quiet = QuietPeriod::OneMonth.days();
    let calibration = Calibration::for_model(HashingEmbedder::MODEL_ID);
    let memory = h.memory();
    let closest = closest_within(&memory, quiet);
    assert!(closest < calibration.threshold, "two wallpapers within the quiet period at {closest:.3}");

    // Echoes: only of originals past the quiet period, recognisably related, with a note.
    let by_id: HashMap<&str, &MemoryRow> = memory.iter().map(|row| (row.id.as_str(), row)).collect();
    let echoes: Vec<&MemoryRow> = memory.iter().filter(|row| row.echo_of.is_some()).collect();
    assert!(echoes.len() >= 10, "only {} echoes in three years", echoes.len());
    for echo in &echoes {
        let original = by_id[echo.echo_of.as_deref().unwrap()];
        assert!(echo.created_at - original.created_at > quiet * DAY, "echo {} came too soon", echo.id);
        let similarity = cosine(&echo.embedding, &original.embedding);
        assert!(calibration.in_echo_band(similarity), "echo {} at {similarity:.3} is out of band", echo.id);
        let generation = h.engine.generation(echo.id.clone()).unwrap();
        assert_eq!(generation.trigger, Trigger::Scheduled);
        assert!(generation.echo_note.as_deref().is_some_and(|note| note.starts_with("Echo of")), "{generation:?}");
    }
    assert_eq!(h.engine.history(HistoryFilter::Echoes, 10_000, 0).unwrap().len(), echoes.len());
    let first_echo = echoes.iter().map(|echo| echo.created_at).min().unwrap();
    assert!(first_echo - START > quiet * DAY, "no echo before anything is past its quiet period");
    assert_eq!(h.engine.stats().least_similar_fallbacks, 0);
}

#[tokio::test]
async fn make_echo_names_its_original_and_lineage_follows() {
    let h = Harness::new();
    h.keywords(&["lighthouse"], &["fog"], &[]);
    let original = h.generate().await;
    h.clock.advance_days(400.0);
    let echo = h.engine.make_echo(original.id.clone(), None).await.expect("echo");
    assert_eq!(echo.trigger, Trigger::EchoRequest);
    assert_eq!(echo.echo_of.as_deref(), Some(original.id.as_str()));
    let note = echo.echo_note.clone().expect("echo note");
    assert!(note.contains(original.concept.title.trim()), "{note}");
    let lineage: Vec<String> = h.engine.lineage(echo.id.clone()).unwrap().into_iter().map(|g| g.id).collect();
    assert_eq!(lineage, [original.id.clone(), echo.id.clone()]);
    assert!(h.engine.describe(echo.id.clone()).unwrap().contains("Echo of"));
    // "Show Original and Echoes" is offered for both, and not for a wallpaper on its own.
    let alone = h.generate().await;
    assert!(h.engine.has_echoes(original.id.clone()).unwrap() && h.engine.has_echoes(echo.id.clone()).unwrap());
    assert!(!h.engine.has_echoes(alone.id.clone()).unwrap());
    assert!(matches!(h.engine.has_echoes("nope".into()), Err(AutoPaperError::NotFound)));

    // Without an id the agent picks one; an unknown id is NotFound.
    let picked = h.engine.generate(Trigger::EchoRequest, None).await.expect("picked echo");
    assert!(picked.echo_of.is_some());
    assert!(matches!(h.engine.make_echo("nope".into(), None).await, Err(AutoPaperError::NotFound)));
}

// ── Budget and failures ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn over_budget_keeps_latest_wallpaper_and_records_why_without_calling_anyone() {
    let h = Harness::new();
    h.keywords(&["lighthouse"], &[], &[]);
    let liked = h.generate().await;
    h.engine.rate(liked.id.clone(), Rating::Liked).unwrap();
    let other = h.generate().await;
    h.engine.mark_shown(other.id.clone()).unwrap();
    h.clock.advance_days(1.0); // the two new wallpapers restarted the schedule
    h.update(|s| {
        s.text_provider.kind = ProviderKind::OpenAi;
        s.image_provider.kind = ProviderKind::OpenAi;
        s.monthly_budget_cents = Some(1);
    });

    assert!(matches!(h.engine.run_if_due(None).await, Err(AutoPaperError::BudgetReached { budget_cents: 1 })));
    assert_eq!(h.engine.current().unwrap().unwrap().id, other.id, "the last generated wallpaper stays displayed");
    let run = h.engine.runs(1, 0).unwrap().remove(0);
    assert_eq!(run.status, RunStatus::Blocked);
    assert!(run.events.is_empty());
    assert!(run.detail.contains("$0.01") && run.detail.contains("No network request"));
    let budget = h.engine.budget_status().unwrap();
    assert!(budget.blocked && budget.next_cost_microusd > 0);
    assert!(budget.message.contains("current wallpaper stays"));
    assert!(h.http.requests().is_empty(), "nothing was paid for");
    assert_eq!(h.engine.next_due().unwrap(), Some(h.now() + DAY), "the slot counts as filled");
    assert_eq!(h.engine.run_if_due(None).await.unwrap().map(|s| s.generation.id), None, "not due again yet");

    assert!(matches!(
        h.engine.generate(Trigger::Manual, None).await,
        Err(AutoPaperError::BudgetReached { budget_cents: 1 })
    ));
    assert_eq!(h.engine.revisit_liked().unwrap().revisit, Some(RevisitReason::Requested));

    h.update(|s| s.fallback = Fallback::KeepCurrent);
    h.clock.advance_days(1.0);
    assert!(matches!(h.engine.run_if_due(None).await, Err(AutoPaperError::BudgetReached { .. })));
    assert_eq!(h.store().memory().unwrap().len(), 2, "no failed attempt was recorded");
}

#[tokio::test]
async fn a_failing_provider_backs_off_and_revisits_once() {
    let h = Harness::new();
    healthy_services(&h.http);
    h.keywords(&["lighthouse"], &[], &[]);
    let liked = h.generate().await;
    h.engine.rate(liked.id.clone(), Rating::Liked).unwrap();
    h.clock.advance_days(1.0);
    h.update(|s| s.text_provider.kind = ProviderKind::OpenAi);
    h.http.always("/v1/responses", 503, br#"{"error":{"message":"overloaded"}}"#.to_vec());

    let shown = h.engine.run_if_due(None).await.unwrap().expect("revisit");
    assert_eq!(shown.revisit, Some(RevisitReason::ProviderFailed));
    assert_eq!(shown.generation.id, liked.id);
    let now = h.now();
    assert_eq!(h.engine.next_due().unwrap(), Some(now + 600), "10 minutes");
    let failed = h.store().generation_count().unwrap();
    assert_eq!(failed, 1, "failed attempts aren't memory");

    h.clock.set(now + 300);
    assert!(h.engine.run_if_due(None).await.unwrap().is_none(), "still backing off");
    h.clock.set(now + 600);
    let second = h.engine.run_if_due(None).await;
    assert!(
        matches!(second, Err(AutoPaperError::ProviderUnavailable { provider: ProviderKind::OpenAi, .. })),
        "{second:?}"
    );
    assert_eq!(h.engine.next_due().unwrap(), Some(now + 600 + 1200), "20 minutes");
    assert!(h.engine.history(HistoryFilter::All, 10, 0).unwrap().iter().all(|g| g.status == GenerationStatus::Ok));

    // The provider recovers: the next attempt makes a wallpaper and the backoff ends.
    h.update(|s| s.text_provider.kind = ProviderKind::Demo);
    h.clock.set(now + 1800);
    let made = h.engine.run_if_due(None).await.unwrap().expect("a new wallpaper");
    assert_eq!(made.revisit, None);
    assert_eq!(made.generation.trigger, Trigger::Scheduled);
    assert_eq!(h.engine.next_due().unwrap(), Some(now + 1800 + DAY));
}

#[tokio::test]
async fn failures_without_anything_liked_are_errors_and_are_recorded() {
    let h = Harness::new();
    healthy_services(&h.http);
    h.update(|s| s.text_provider.kind = ProviderKind::OpenAi);
    h.http.always("/v1/responses", 401, br#"{"error":{"message":"bad key sk-live-0123456789abcdefghijkl"}}"#.to_vec());
    let error = h.engine.run_if_due(None).await.unwrap_err();
    assert!(matches!(error, AutoPaperError::InvalidKey { provider: ProviderKind::OpenAi }), "{error:?}");
    assert!(h.engine.next_due().unwrap().unwrap() > h.now(), "even a key problem backs off");

    // The failed attempt is stored (not in history or memory), with no secret in it.
    assert_eq!(h.store().generation_count().unwrap(), 0);
    assert!(h.engine.history(HistoryFilter::All, 10, 0).unwrap().is_empty());
    let rows = failed_rows(&h);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "failed");
    assert!(!rows[0].1.contains("sk-"), "{:?}", rows[0]);

    // A missing key is recorded nowhere: nothing was attempted.
    h.update(|s| s.image_provider.kind = ProviderKind::Google);
    h.update(|s| s.text_provider.kind = ProviderKind::Google);
    let error = h.engine.generate(Trigger::Manual, None).await.unwrap_err();
    assert!(matches!(error, AutoPaperError::MissingKey { provider: ProviderKind::Google }), "{error:?}");
    assert_eq!(failed_rows(&h).len(), 1, "nothing new recorded");
}

#[tokio::test]
async fn a_missing_painting_key_stops_before_an_idea_is_paid_for() {
    // Both roles are checked, but no paid work starts when the painter's key is missing.
    let h = Harness::new();
    h.keywords(&["harbour"], &[], &[]);
    h.update(|s| {
        s.text_provider.kind = ProviderKind::OpenAi;
        s.image_provider.kind = ProviderKind::Google;
    });
    healthy_services(&h.http);
    let stages = Arc::new(Stages::default());
    let error = h.engine.generate(Trigger::Manual, Some(stages.clone())).await.unwrap_err();
    assert!(matches!(error, AutoPaperError::MissingKey { provider: ProviderKind::Google }), "{error:?}");
    assert!(work_requests(&h.http).is_empty(), "no paid request was sent");
    assert_eq!(h.http.requests().len(), 1, "the writer availability check still ran");
    assert_eq!(*stages.0.lock().unwrap(), [ProgressStage::CheckingServices]);
    assert_eq!(h.engine.spend_summary().unwrap().spent_microusd, 0);

    // The same for the writer's key, which is checked first (it's called first).
    let keyless = Harness::new();
    keyless.update(|s| s.text_provider.kind = ProviderKind::Google);
    let error = keyless.engine.generate(Trigger::Manual, None).await.unwrap_err();
    assert!(matches!(error, AutoPaperError::MissingKey { provider: ProviderKind::Google }), "{error:?}");
    assert!(keyless.http.requests().is_empty());
}

/// (status, error) of every generation that isn't ok, oldest first.
fn failed_rows(h: &Harness) -> Vec<(String, String)> {
    let conn = rusqlite::Connection::open(h.dir.path().join("autopaper.sqlite3")).unwrap();
    let mut statement =
        conn.prepare("SELECT status, error FROM generations WHERE status <> 'ok' ORDER BY created_at, rowid").unwrap();
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get::<_, Option<String>>(1)?.unwrap_or_default())))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

// ── Cancel ──────────────────────────────────────────────────────────────────────────────────

struct CancelWhileComposing(Mutex<Option<Arc<Engine>>>);

impl ProgressObserver for CancelWhileComposing {
    fn on_progress(&self, stage: ProgressStage) {
        if stage == ProgressStage::Composing
            && let Some(engine) = &*self.0.lock().unwrap()
        {
            engine.cancel();
        }
    }
}

#[tokio::test]
async fn cancel_stops_at_the_next_stage() {
    let h = Harness::new();
    h.keywords(&["lighthouse"], &[], &[]);
    let canceller = Arc::new(CancelWhileComposing(Mutex::new(Some(h.engine.clone()))));
    let observer: Arc<dyn ProgressObserver> = canceller.clone();
    let result = h.engine.generate(Trigger::Manual, Some(observer)).await;
    assert!(matches!(result, Err(AutoPaperError::Cancelled)), "{result:?}");
    *canceller.0.lock().unwrap() = None;
    assert!(h.engine.history(HistoryFilter::All, 10, 0).unwrap().is_empty());
    assert!(failed_rows(&h).is_empty(), "a cancel isn't a failure");
    assert_eq!(h.engine.storage_usage().unwrap().images_on_disk, 0);

    // A cancel applies to the generation in progress only.
    h.engine.cancel();
    h.generate().await;
}

// ── Storage ─────────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn prune_never_removes_liked_or_current_images() {
    let h = Harness::new();
    h.keywords(&["lighthouse"], &[], &[]);
    let mut made = Vec::new();
    for _ in 0..5 {
        made.push(h.generate().await);
        h.clock.advance_days(1.0);
    }
    h.engine.rate(made[1].id.clone(), Rating::Liked).unwrap();
    h.engine.mark_shown(made[3].id.clone()).unwrap();
    let display = DisplayTarget { id: "main".into(), width: 64, height: 40 };
    let old_render = h.engine.render_for_display(made[0].id.clone(), display.clone()).unwrap();
    let current_render = h.engine.render_for_display(made[3].id.clone(), display).unwrap();

    // Over the (minimum) limit: a sparse 300 MB file takes no real disk space.
    let filler = h.dir.path().join("thumbs").join("filler.bin");
    std::fs::File::create(&filler).unwrap().set_len(300 * 1024 * 1024).unwrap();
    h.update(|s| s.storage_limit_mb = 1);
    assert_eq!(h.engine.settings().unwrap().storage_limit_mb, 256, "clamped");
    h.engine.prune().unwrap();

    for (index, generation) in made.iter().enumerate() {
        let after = h.engine.generation(generation.id.clone()).unwrap();
        let kept = index == 1 || index == 3;
        assert_eq!(after.image_path.is_some(), kept, "#{index}");
        assert_eq!(exists(&generation.image_path), kept, "#{index} file");
        assert!(exists(&after.thumb_path), "#{index} keeps its thumbnail");
    }
    assert!(!Path::new(&old_render).exists() && Path::new(&current_render).exists());
    assert_eq!(h.memory().len(), 5, "memory is kept");
    assert!(matches!(
        h.engine.render_for_display(made[0].id.clone(), DisplayTarget { id: "m".into(), width: 64, height: 40 }),
        Err(AutoPaperError::NotFound)
    ));

    // Deleting one removes its files and its row; the one on the desktop keeps its render while it's there.
    h.engine.delete_generation(made[3].id.clone()).unwrap();
    assert!(!exists(&made[3].image_path) && Path::new(&current_render).exists());
    assert!(matches!(h.engine.generation(made[3].id.clone()), Err(AutoPaperError::NotFound)));
    let liked_render =
        h.engine.render_for_display(made[1].id.clone(), DisplayTarget { id: "m".into(), width: 64, height: 40 }).unwrap();
    h.engine.delete_generation(made[1].id.clone()).unwrap();
    assert!(!exists(&made[1].image_path) && !Path::new(&liked_render).exists(), "not on the desktop: all of it goes");
}

#[tokio::test]
async fn clear_history_keeping_memory_or_forgetting_everything() {
    let h = Harness::new();
    h.keywords(&["lighthouse"], &[], &[]);
    let first = h.generate().await;
    let second = h.generate().await;
    h.engine.rate(first.id.clone(), Rating::Liked).unwrap();
    h.engine.mark_shown(second.id.clone()).unwrap();

    h.engine.clear_history(true).unwrap();
    assert!(h.engine.history(HistoryFilter::All, 10, 0).unwrap().is_empty());
    assert_eq!(h.engine.current().unwrap(), None);
    assert!(!exists(&first.image_path) && !exists(&second.thumb_path));
    assert_eq!(h.engine.storage_usage().unwrap().images_on_disk, 0);
    assert_eq!(h.memory().len(), 2, "memory kept, so repeats are still avoided");
    assert_eq!(h.engine.taste_summary().unwrap().ratings, 1);
    assert!(matches!(h.engine.revisit_liked(), Err(AutoPaperError::NothingToRevisit)));
    let third = h.generate().await;
    assert_eq!(h.engine.history(HistoryFilter::All, 10, 0).unwrap().len(), 1);
    assert!(exists(&third.image_path));

    h.engine.clear_history(false).unwrap();
    assert!(h.memory().is_empty());
    assert_eq!(h.engine.taste_summary().unwrap().ratings, 0);
    assert_eq!(h.engine.keywords().unwrap().len(), 1, "keywords stay");
    assert_eq!(h.engine.storage_usage().unwrap().generations, 0);
}

// ── Embedding model changes ─────────────────────────────────────────────────────────────────

/// The hashing embedder under another name: stands in for "the model that made these embeddings".
struct OldModel;

impl Embedder for OldModel {
    fn model_id(&self) -> &str {
        "old-model"
    }

    fn embed(&self, text: &str) -> autopaper_core::error::Result<Vec<f32>> {
        let mut vector = HashingEmbedder.embed(text)?;
        vector.reverse();
        Ok(vector)
    }
}

#[tokio::test]
async fn memory_is_re_embedded_lazily_after_a_model_change() {
    let mut h = Harness::with(Arc::new(OldModel), 3);
    h.keywords(&["lighthouse"], &["fog"], &[]);
    for _ in 0..3 {
        h.generate().await;
    }
    assert!(h.memory().iter().all(|row| row.embedding_model == "old-model"));

    h.reopen(Arc::new(HashingEmbedder));
    assert!(h.memory().iter().all(|row| row.embedding_model == "old-model"), "nothing happens on open");
    h.generate().await;
    let memory = h.memory();
    assert_eq!(memory.len(), 4);
    assert!(memory.iter().all(|row| row.embedding_model == HashingEmbedder::MODEL_ID));
    assert_eq!(h.engine.stats().reembedded, 3);
    let expected = HashingEmbedder
        .embed(&autopaper_core::embed::concept_text(&h.engine.generation(memory[0].id.clone()).unwrap().concept))
        .unwrap();
    assert_eq!(memory[0].embedding, expected);
}

// ── Scheduling ──────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_schedule_follows_cadence_and_pause() {
    let h = Harness::new();
    h.update(|s| s.cadence = Cadence::Every6Hours);
    assert_eq!(h.engine.next_due().unwrap(), Some(START), "due at once when nothing was scheduled");
    let made = h.engine.run_if_due(None).await.unwrap().expect("made");
    assert_eq!(made.generation.trigger, Trigger::Scheduled);
    assert_eq!(h.engine.next_due().unwrap(), Some(START + 6 * 3600));
    assert!(h.engine.run_if_due(None).await.unwrap().is_none());

    h.update(|s| s.paused = true);
    h.clock.advance_days(1.0);
    assert_eq!(h.engine.next_due().unwrap(), None);
    assert!(h.engine.run_if_due(None).await.unwrap().is_none());
    h.update(|s| {
        s.paused = false;
        s.cadence = Cadence::Manual;
    });
    assert_eq!(h.engine.next_due().unwrap(), None);
    assert!(h.engine.run_if_due(None).await.unwrap().is_none());
}

#[tokio::test]
async fn every_new_wallpaper_restarts_the_schedule_but_revisits_dont() {
    const HOUR: i64 = 3600;
    let h = Harness::new();
    h.keywords(&["lighthouse"], &[], &[]);
    h.update(|s| s.cadence = Cadence::Every6Hours);
    let first = h.engine.run_if_due(None).await.unwrap().expect("made").generation;
    h.engine.rate(first.id.clone(), Rating::Liked).unwrap();
    assert_eq!(h.engine.next_due().unwrap(), Some(START + 6 * HOUR));

    for (hours, trigger) in [(2, Trigger::Manual), (3, Trigger::DislikeReplace), (4, Trigger::EchoRequest)] {
        h.clock.set(START + hours * HOUR);
        let made = h.engine.generate(trigger, None).await.unwrap();
        assert_eq!(made.trigger, trigger);
        assert_eq!(h.engine.next_due().unwrap(), Some(START + (hours + 6) * HOUR), "{trigger:?} restarts it");
    }
    let due = START + 10 * HOUR;

    // Bringing back a liked one isn't a new wallpaper, nor is an attempt that failed.
    h.clock.set(START + 5 * HOUR);
    let revisit = h.engine.revisit_liked().unwrap();
    h.engine.mark_shown(revisit.generation.id).unwrap();
    assert_eq!(h.engine.next_due().unwrap(), Some(due), "a revisit");
    h.update(|s| s.text_provider.kind = ProviderKind::OpenAi); // nothing answers: Offline
    assert!(h.engine.generate(Trigger::Manual, None).await.is_err());
    assert_eq!(h.engine.next_due().unwrap(), Some(due), "a failed attempt");

    h.update(|s| s.text_provider.kind = ProviderKind::Demo);
    h.clock.set(due - 1);
    assert!(h.engine.run_if_due(None).await.unwrap().is_none());
    h.clock.set(due);
    let made = h.engine.run_if_due(None).await.unwrap().expect("due one interval after the echo");
    assert_eq!(made.generation.trigger, Trigger::Scheduled);
}

#[tokio::test]
async fn a_database_without_a_schedule_record_counts_from_its_newest_wallpaper() {
    let mut h = Harness::new();
    h.keywords(&["lighthouse"], &[], &[]);
    h.engine.generate(Trigger::Manual, None).await.unwrap();
    // As a database written before every new wallpaper was recorded: no record, one manual wallpaper.
    let conn = rusqlite::Connection::open(h.dir.path().join("autopaper.sqlite3")).unwrap();
    conn.execute("DELETE FROM state WHERE key = 'last_slot_at'", []).unwrap();
    h.reopen(Arc::new(HashingEmbedder));
    assert_eq!(h.engine.next_due().unwrap(), Some(START + DAY));
}

#[tokio::test]
async fn failed_attempts_are_forgotten_after_30_days() {
    let h = Harness::new();
    healthy_services(&h.http);
    h.keywords(&["lighthouse"], &[], &[]);
    h.update(|s| s.text_provider.kind = ProviderKind::OpenAi); // nothing answers: Offline
    for day in [0.0, 20.0] {
        h.clock.advance_days(day);
        assert!(h.engine.generate(Trigger::Manual, None).await.is_err());
    }
    assert_eq!(failed_rows(&h).len(), 2);

    h.clock.set(START + 31 * DAY);
    h.engine.prune().unwrap();
    assert_eq!(failed_rows(&h).len(), 1, "the one from 31 days ago goes; the 11-day-old one stays");

    // A new wallpaper's own pruning does the same.
    h.update(|s| s.text_provider.kind = ProviderKind::Demo);
    h.clock.set(START + 51 * DAY);
    let made = h.generate().await;
    assert!(failed_rows(&h).is_empty());
    assert_eq!(h.engine.history(HistoryFilter::All, 10, 0).unwrap(), [h.engine.generation(made.id).unwrap()]);
}

/// Like the real client when a hosted API never answers: the request's whole timeout passes, then `Offline`.
struct NeverAnswers;

#[async_trait::async_trait]
impl HttpClient for NeverAnswers {
    async fn send(&self, request: HttpRequest) -> autopaper_core::error::Result<HttpResponse> {
        tokio::time::sleep(std::time::Duration::from_secs(request.timeout_secs)).await;
        Err(AutoPaperError::Offline)
    }
}

#[tokio::test(start_paused = true)]
async fn a_hosted_api_that_never_answers_is_a_provider_failure_not_offline() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(FixedClock::at(START));
    let engine = open(dir.path(), clock.clone(), Arc::new(NeverAnswers), Arc::new(HashingEmbedder), 7);
    engine.add_keyword("lighthouse".into(), KeywordWeight::Must).unwrap();
    let liked = engine.generate(Trigger::Manual, None).await.unwrap();
    engine.rate(liked.id.clone(), Rating::Liked).unwrap();
    clock.advance_days(1.0);
    let mut settings = engine.settings().unwrap();
    settings.text_provider.kind = ProviderKind::OpenAi;
    engine.update_settings(settings).unwrap();

    let shown = engine.run_if_due(None).await.unwrap().expect("a revisit");
    assert_eq!(shown.revisit, Some(RevisitReason::ServicesUnavailable), "bounded service preflight");
    clock.set(clock.now() + 600);
    let again = engine.run_if_due(None).await.unwrap().unwrap();
    assert_eq!(again.revisit, Some(RevisitReason::ServicesUnavailable));
    let run = engine.run(engine.runs(1, 0).unwrap()[0].id.clone()).unwrap();
    assert!(run.events.iter().any(|event| event.kind == "service_check" && event.detail.contains("8 seconds")));

}

#[tokio::test]
async fn providers_are_checked_with_a_cheap_call() {
    let h = Harness::new();
    let demo = ProviderSelection { kind: ProviderKind::Demo, model: String::new(), base_url: None };
    h.engine.test_provider(demo.clone(), autopaper_core::ProviderJob::Images).await.unwrap();
    assert_eq!(h.engine.list_models(demo, autopaper_core::ProviderJob::Concepts).await.unwrap()[0].id, "demo");

    let openai = ProviderSelection { kind: ProviderKind::OpenAi, model: String::new(), base_url: None };
    h.http.once("/v1/models", 401, br#"{"error":{"message":"Incorrect API key"}}"#.to_vec());
    let error = h.engine.test_provider(openai, autopaper_core::ProviderJob::Concepts).await.unwrap_err();
    assert!(matches!(error, AutoPaperError::InvalidKey { provider: ProviderKind::OpenAi }), "{error:?}");

    let ollama = ProviderSelection { kind: ProviderKind::Ollama, model: String::new(), base_url: None };
    let images = h.engine.test_provider(ollama, autopaper_core::ProviderJob::Images).await.unwrap_err();
    assert!(matches!(images, AutoPaperError::Unsupported { provider: ProviderKind::Ollama, .. }), "{images:?}");
}

// ── Unreadable answers ──────────────────────────────────────────────────────────────────────

/// One harbour candidate (valid for the Must "lighthouse").
fn harbour() -> serde_json::Value {
    candidate(
        "Harbour lighthouse at dusk",
        "A white lighthouse on a stone pier at dusk, gulls over calm water.",
        "a fishing harbour",
        &["white lighthouse", "stone pier", "gulls", "moored boats"],
        &["amber", "slate blue", "white"],
        "An oil painting of a white lighthouse on a stone pier in a fishing harbour at dusk, gulls over calm water.",
    )
}

#[tokio::test]
async fn an_unreadable_answer_is_asked_again_and_its_cost_still_counts() {
    let h = Harness::new();
    healthy_services(&h.http);
    h.keywords(&["lighthouse"], &[], &[]);
    h.update(|s| s.text_provider.kind = ProviderKind::OpenAi);
    let whole = json!({ "candidates": [harbour()] }).to_string();
    // Cut off by the output limit, then complete but broken JSON, then a good answer.
    h.http.once("/v1/responses", 200, responses_text("incomplete", &whole[..40]));
    h.http.once("/v1/responses", 200, responses_text("completed", &whole[..whole.len() / 2]));
    h.http.once("/v1/responses", 200, responses(&[harbour()]));
    let made = h.generate().await;
    assert_eq!(made.concept.title, "Harbour lighthouse at dusk");
    assert_eq!(work_requests(&h.http).len(), 3);
    let stats = h.engine.stats();
    assert_eq!((stats.compose_calls, stats.invalid_retries), (3, 2), "{stats:?}");
    // Every answer was billed, readable or not.
    let usage = autopaper_core::providers::Usage { input_tokens: 1800, output_tokens: 1500 };
    let one_call = autopaper_core::pricing::text_cost(ProviderKind::OpenAi, "gpt-6-luna-2026-09-22", &usage);
    assert!(one_call > 0);
    assert_eq!(made.cost_microusd, 3 * one_call);
    assert_eq!(h.engine.spend_summary().unwrap().spent_microusd, 3 * one_call);

    // Unreadable every time: after three asks it fails, and says why.
    h.http.always("/v1/responses", 200, responses_text("completed", "Here are four ideas: rain over ruins"));
    let error = h.engine.generate(Trigger::Manual, None).await.unwrap_err();
    assert!(
        matches!(&error, AutoPaperError::InvalidResponse { detail } if detail.contains("couldn't be read")),
        "{error:?}"
    );
    assert_eq!(work_requests(&h.http).len(), 6);
    assert_eq!(h.engine.spend_summary().unwrap().spent_microusd, 6 * one_call);
}

/// A candidate that leaves out the Must "lighthouse".
fn no_lighthouse() -> serde_json::Value {
    candidate(
        "Quiet harbour at dusk",
        "Moored boats in a calm harbour at dusk.",
        "a fishing harbour",
        &["moored boats", "stone pier"],
        &["amber", "slate blue"],
        "An oil painting of moored fishing boats beside a stone pier in a calm harbour at dusk, gulls over the water.",
    )
}

#[tokio::test]
async fn a_retry_tells_the_model_what_it_missed_and_the_error_names_it() {
    let h = Harness::new();
    healthy_services(&h.http);
    h.keywords(&["lighthouse"], &[], &[]);
    h.update(|s| s.text_provider.kind = ProviderKind::OpenAi);
    h.http.once("/v1/responses", 200, responses(&[no_lighthouse(), no_lighthouse()]));
    h.http.once("/v1/responses", 200, responses(&[harbour()]));
    let made = h.generate().await;
    assert_eq!(made.concept.title, "Harbour lighthouse at dusk");
    let first = work_json(&h.http, 0).to_string();
    let second = work_json(&h.http, 1).to_string();
    assert!(!first.contains("couldn't be used"), "{first}");
    assert!(second.contains("The prompt left out the Must keyword \u{201c}lighthouse\u{201d}"), "{second}");

    // Missed every time: the error names the keyword and its mood, for the host's link to it.
    h.http.always("/v1/responses", 200, responses(&[no_lighthouse()]));
    let error = h.engine.generate(Trigger::Manual, None).await.unwrap_err();
    let mood = h.engine.moods().unwrap().into_iter().find(|mood| mood.active).unwrap();
    match error {
        AutoPaperError::KeywordNotFollowed { keyword, weight, mood_id } => {
            assert_eq!((keyword.as_str(), weight, mood_id), ("lighthouse", KeywordWeight::Must, mood.id));
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_local_writer_is_given_ten_minutes_for_a_cold_start() {
    let h = Harness::new();
    healthy_services(&h.http);
    h.keywords(&["lighthouse"], &[], &[]);
    h.update(|s| s.text_provider = ProviderSelection { kind: ProviderKind::Ollama, model: "m".into(), base_url: None });
    let chat = json!({
        "model": "m",
        "message": { "role": "assistant", "content": json!({ "candidates": [harbour()] }).to_string() },
        "done": true, "done_reason": "stop", "prompt_eval_count": 10, "eval_count": 10
    });
    h.http.once("/api/chat", 200, chat.to_string());
    h.generate().await;
    let chats: Vec<_> = h.http.requests().into_iter().filter(|request| request.url.ends_with("/api/chat")).collect();
    assert_eq!(chats.len(), 1);
    assert_eq!(chats[0].timeout_secs, 600, "nothing learned yet: Ollama may take 10 minutes to load a model and answer");
}

struct FakeSystemModel {
    answer: Mutex<String>,
    asked: Mutex<Vec<(String, u32)>>,
    available: bool,
}

impl SystemModel for FakeSystemModel {
    fn status(&self) -> SystemModelStatus {
        SystemModelStatus {
            available: self.available,
            reason: (!self.available).then_some(SystemModelReason::NotEnabled),
            name: "Apple Intelligence".into(),
            context_tokens: 4096,
        }
    }
    fn compose(&self, system: String, _user: String, schema_json: String, _temperature: f32, _max: u32) -> SystemComposeOutcome {
        let max_items = serde_json::from_str::<serde_json::Value>(&schema_json).unwrap()["properties"]["candidates"]["maxItems"].as_u64().unwrap() as u32;
        self.asked.lock().unwrap().push((system, max_items));
        SystemComposeOutcome { json: self.answer.lock().unwrap().clone(), input_tokens: 700, output_tokens: 400, problem: None }
    }
}

#[tokio::test]
async fn the_built_in_model_writes_an_idea_the_composer_repairs_and_the_engine_uses() {
    let h = Harness::new();
    healthy_services(&h.http);
    h.keywords(&["misty mountains"], &[], &["people"]);
    // The model leaves the Must keyword out and denies the Avoid one, as Apple's model does.
    let idea = candidate(
        "Fog ridge",
        "A ridge in low fog.",
        "a high valley",
        &["ridge", "fog"],
        &["grey", "slate"],
        "A wide landscape photograph of a distant ridge in low cloud, no people, calm light.",
    );
    let model = Arc::new(FakeSystemModel { answer: Mutex::new(json!({ "candidates": [idea] }).to_string()), asked: Mutex::new(Vec::new()), available: true });
    h.engine.set_system_model(Some(model.clone()));
    h.update(|s| s.text_provider = ProviderSelection { kind: ProviderKind::System, model: String::new(), base_url: None });
    let made = h.generate().await;
    assert!(made.concept.prompt.starts_with("misty mountains, a wide landscape photograph"), "{}", made.concept.prompt);
    assert!(!made.concept.prompt.contains("people"), "{}", made.concept.prompt);
    let asked = model.asked.lock().unwrap();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].1, 2, "a 4,096-token window asks for two ideas");
    assert!(asked[0].0.len() < 2000, "short instructions: {}", asked[0].0.len());
    let stored = h.engine.generation(made.id.clone()).unwrap();
    assert_eq!(stored.text_provider, ProviderKind::System);
    assert_eq!(stored.cost_microusd, 0);
}

#[tokio::test]
async fn an_unavailable_built_in_model_stops_before_anything_is_made() {
    let h = Harness::new();
    healthy_services(&h.http);
    h.keywords(&["lighthouse"], &[], &[]);
    let model = Arc::new(FakeSystemModel { answer: Mutex::new(String::new()), asked: Mutex::new(Vec::new()), available: false });
    h.engine.set_system_model(Some(model.clone()));
    h.update(|s| s.text_provider = ProviderSelection { kind: ProviderKind::System, model: String::new(), base_url: None });
    let error = h.engine.generate(Trigger::Manual, None).await.expect_err("unavailable");
    assert!(matches!(error, AutoPaperError::ProviderUnavailable { provider: ProviderKind::System, .. }), "{error:?}");
    assert!(model.asked.lock().unwrap().is_empty());
}

#[tokio::test]
async fn control_characters_from_a_provider_never_reach_the_database() {
    let h = Harness::new();
    healthy_services(&h.http);
    h.keywords(&["lighthouse"], &[], &[]);
    h.update(|s| s.text_provider = ProviderSelection { kind: ProviderKind::Ollama, model: "m".into(), base_url: None });
    let idea = candidate(
        "Lighthouse\u{1b}[2J at dusk",
        "A lighthouse\u{1b}]52;c;QUJD\u{7} at dusk.",
        "a harbour\r",
        &["lighthouse\u{1b}[31m", "pier"],
        &["amber"],
        "An oil painting of a lighthouse at dusk.",
    );
    let chat = json!({
        "model": "m\u{1b}]52;c;QUJD\u{7}",
        "message": { "role": "assistant", "content": json!({ "candidates": [idea] }).to_string() },
        "done": true, "done_reason": "stop", "prompt_eval_count": 10, "eval_count": 10
    });
    h.http.once("/api/chat", 200, chat.to_string());
    let made = h.generate().await;
    let stored = h.engine.generation(made.id.clone()).unwrap();
    let concept = &stored.concept;
    let texts = [&concept.title, &concept.summary, &concept.setting, &stored.text_model]
        .into_iter()
        .chain(&concept.elements);
    for text in texts {
        assert!(!text.chars().any(char::is_control), "{text:?}");
    }
    assert_eq!(stored.text_model, "m", "a model name that isn't plausible falls back to the one asked for");
}

// ── Scheduling: backoff, Retry-After, cancel, clock changes ────────────────────────────────

/// Every request takes `secs` of the (fake) clock and then fails, like a local server that hangs.
struct SlowFailure {
    clock: Arc<FixedClock>,
    secs: i64,
}

#[async_trait::async_trait]
impl HttpClient for SlowFailure {
    async fn send(&self, _request: HttpRequest) -> autopaper_core::error::Result<HttpResponse> {
        self.clock.set(self.clock.now() + self.secs);
        Err(AutoPaperError::Offline)
    }
}

#[tokio::test]
async fn a_slow_failure_backs_off_from_when_it_failed() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(FixedClock::at(START));
    let http = Arc::new(SlowFailure { clock: clock.clone(), secs: 700 });
    let engine = open(dir.path(), clock.clone(), http, Arc::new(HashingEmbedder), 7);
    engine.add_keyword("lighthouse".into(), KeywordWeight::Must).unwrap();
    let mut settings = engine.settings().unwrap();
    settings.text_provider = ProviderSelection { kind: ProviderKind::Ollama, model: "m".into(), base_url: None };
    engine.update_settings(settings).unwrap();

    let error = engine.run_if_due(None).await.unwrap_err();
    assert!(error.is_transient(), "{error:?}");
    let failed_at = clock.now();
    assert!(failed_at > START, "the attempt took time");
    assert_eq!(engine.next_due().unwrap(), Some(failed_at + 600), "10 minutes from the failure, not the start");
}

#[tokio::test]
async fn a_rate_limit_waits_at_least_as_long_as_the_provider_asks() {
    let h = Harness::new();
    healthy_services(&h.http);
    h.keywords(&["lighthouse"], &[], &[]);
    h.update(|s| s.text_provider.kind = ProviderKind::OpenAi);
    let limited = |seconds: &str| HttpResponse {
        status: 429,
        headers: vec![("retry-after".into(), seconds.into())],
        body: br#"{"error":{"message":"slow down"}}"#.to_vec(),
    };
    h.http.once_response("/v1/responses", limited("1800"));
    let error = h.engine.run_if_due(None).await.unwrap_err();
    assert!(matches!(error, AutoPaperError::RateLimited { retry_after_secs: 1800, .. }), "{error:?}");
    assert_eq!(h.engine.next_due().unwrap(), Some(h.now() + 1800), "Retry-After, not 10 minutes");

    // A shorter Retry-After doesn't shorten the backoff.
    h.clock.set(h.now() + 1800);
    h.http.once_response("/v1/responses", limited("5"));
    assert!(h.engine.run_if_due(None).await.is_err());
    assert_eq!(h.engine.next_due().unwrap(), Some(h.now() + 1200), "the second failure's 20 minutes");
}

#[tokio::test]
async fn cancelling_a_scheduled_run_isnt_a_failure() {
    let h = Harness::new();
    healthy_services(&h.http);
    h.keywords(&["lighthouse"], &[], &[]);
    let liked = h.generate().await;
    h.engine.rate(liked.id.clone(), Rating::Liked).unwrap();
    h.clock.advance_days(1.0);

    let canceller = Arc::new(CancelWhileComposing(Mutex::new(Some(h.engine.clone()))));
    let observer: Arc<dyn ProgressObserver> = canceller.clone();
    let result = h.engine.run_if_due(Some(observer)).await;
    *canceller.0.lock().unwrap() = None;
    assert!(matches!(result, Err(AutoPaperError::Cancelled)), "{result:?}");
    assert_eq!(
        h.engine.next_due().unwrap(),
        Some(h.now() + DAY),
        "the slot is filled (a host timer won't start it again at once), with no backoff"
    );
    assert!(h.engine.run_if_due(None).await.unwrap().is_none());

    // The first real failure afterwards still revisits a liked wallpaper.
    h.clock.advance_days(1.0);
    h.update(|s| s.text_provider.kind = ProviderKind::OpenAi);
    h.http.always("/v1/responses", 503, br#"{"error":{"message":"overloaded"}}"#.to_vec());
    let shown = h.engine.run_if_due(None).await.unwrap().expect("a revisit");
    assert_eq!(shown.revisit, Some(RevisitReason::ProviderFailed));
}

/// Holds a generation at Composing (blocking its thread) until released.
struct HoldAtComposing {
    reached: Mutex<Option<mpsc::Sender<()>>>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl ProgressObserver for HoldAtComposing {
    fn on_progress(&self, stage: ProgressStage) {
        if stage == ProgressStage::Composing
            && let Some(reached) = self.reached.lock().unwrap().take()
        {
            reached.send(()).unwrap();
            self.release.lock().unwrap().recv().unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_also_stops_a_generation_waiting_for_its_turn() {
    let h = Harness::new();
    h.keywords(&["lighthouse"], &[], &[]);
    let (reached_tx, reached_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let holder: Arc<dyn ProgressObserver> =
        Arc::new(HoldAtComposing { reached: Mutex::new(Some(reached_tx)), release: Mutex::new(release_rx) });
    let scheduled = {
        let engine = h.engine.clone();
        tokio::spawn(async move { engine.run_if_due(Some(holder)).await })
    };
    tokio::task::spawn_blocking(move || reached_rx.recv()).await.unwrap().unwrap();

    // "New Wallpaper Now" queues behind the scheduled run, and the person cancels.
    let mut manual = Box::pin(h.engine.generate(Trigger::Manual, None));
    std::future::poll_fn(|cx| {
        assert!(manual.as_mut().poll(cx).is_pending(), "waits for the scheduled run");
        Poll::Ready(())
    })
    .await;
    h.engine.cancel();
    release_tx.send(()).unwrap();

    let manual = manual.await;
    assert!(matches!(manual, Err(AutoPaperError::Cancelled)), "the queued one too: {manual:?}");
    let scheduled = scheduled.await.unwrap();
    assert!(matches!(scheduled, Err(AutoPaperError::Cancelled)), "{scheduled:?}");
    assert!(h.engine.history(HistoryFilter::All, 10, 0).unwrap().is_empty());
    // Requests made after the cancel aren't affected.
    h.generate().await;
}

/// An HTTP client whose requests never answer (a server that accepted the connection and hung), noting each
/// request as it starts.
#[derive(Default)]
struct Hangs {
    started: tokio::sync::Notify,
    requests: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl HttpClient for Hangs {
    async fn send(&self, request: HttpRequest) -> autopaper_core::Result<HttpResponse> {
        self.requests.lock().unwrap().push(request.url);
        self.started.notify_one();
        std::future::pending().await
    }
}

#[tokio::test]
async fn cancel_drops_a_provider_request_that_hangs() {
    let dir = tempfile::tempdir().unwrap();
    let hangs = Arc::new(Hangs::default());
    let engine = open(dir.path(), Arc::new(FixedClock::at(START)), hangs.clone(), Arc::new(HashingEmbedder), 7);
    engine.add_keyword("lighthouse".into(), KeywordWeight::Must).unwrap();
    let mut settings = engine.settings().unwrap();
    settings.text_provider = ProviderSelection {
        kind: ProviderKind::OpenAiCompatible,
        model: "m".into(),
        base_url: Some("http://127.0.0.1:1234/v1".into()),
    };
    engine.update_settings(settings).unwrap();

    let running = tokio::spawn({
        let engine = engine.clone();
        async move { engine.generate(Trigger::Manual, None).await }
    });
    hangs.started.notified().await;
    engine.cancel();
    // OpenAI-compatible text waits up to 180 s; the cancel doesn't.
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), running).await.expect("cancelled promptly").unwrap();
    assert!(matches!(result, Err(AutoPaperError::Cancelled)), "{result:?}");
    assert_eq!(hangs.requests.lock().unwrap().len(), 1, "{:?}", hangs.requests.lock().unwrap());
    assert!(engine.history(HistoryFilter::All, 10, 0).unwrap().is_empty());
    let conn = rusqlite::Connection::open(dir.path().join("autopaper.sqlite3")).unwrap();
    let rows: i64 = conn.query_row("SELECT COUNT(*) FROM generations", [], |row| row.get(0)).unwrap();
    assert_eq!(rows, 0, "a cancel isn't a failure");

    // Waiting for its turn behind a hung request, a generation is cancelled at once too.
    let first = tokio::spawn({
        let engine = engine.clone();
        async move { engine.generate(Trigger::Manual, None).await }
    });
    hangs.started.notified().await;
    let waiting = tokio::spawn({
        let engine = engine.clone();
        async move { engine.run_if_due(None).await }
    });
    for _ in 0..10 {
        tokio::task::yield_now().await; // the scheduled run notes its ticket and waits for its turn
    }
    engine.cancel();
    let first = tokio::time::timeout(std::time::Duration::from_secs(5), first).await.unwrap().unwrap();
    let waiting = tokio::time::timeout(std::time::Duration::from_secs(5), waiting).await.unwrap().unwrap();
    assert!(matches!(first, Err(AutoPaperError::Cancelled)), "{first:?}");
    assert!(matches!(waiting, Err(AutoPaperError::Cancelled)), "{waiting:?}");
    assert_eq!(engine.next_due().unwrap(), Some(START + DAY), "the cancelled scheduled run filled its slot");
}

/// A scripted client that cancels the engine's generation as the text model answers: the answer and the cancel
/// arrive together.
struct CancelAsItAnswers {
    http: StubHttp,
    engine: Mutex<Option<Arc<Engine>>>,
}

#[async_trait::async_trait]
impl HttpClient for CancelAsItAnswers {
    async fn send(&self, request: HttpRequest) -> autopaper_core::Result<HttpResponse> {
        if request.url.ends_with("/v1/responses") && let Some(engine) = &*self.engine.lock().unwrap() {
            engine.cancel();
        }
        self.http.send(request).await
    }
}

#[tokio::test]
async fn an_answer_that_arrives_with_the_cancel_still_counts_its_cost() {
    let dir = tempfile::tempdir().unwrap();
    let client = Arc::new(CancelAsItAnswers { http: StubHttp::new(), engine: Mutex::new(None) });
    healthy_services(&client.http);
    client.http.once("/v1/responses", 200, responses(&[harbour()]));
    let engine = open(dir.path(), Arc::new(FixedClock::at(START)), client.clone(), Arc::new(HashingEmbedder), 7);
    engine.add_keyword("lighthouse".into(), KeywordWeight::Must).unwrap();
    let mut settings = engine.settings().unwrap();
    settings.text_provider.kind = ProviderKind::OpenAi;
    engine.update_settings(settings).unwrap();
    *client.engine.lock().unwrap() = Some(engine.clone());

    let result = engine.generate(Trigger::Manual, None).await;
    *client.engine.lock().unwrap() = None;
    assert!(matches!(result, Err(AutoPaperError::Cancelled)), "{result:?}");
    let usage = autopaper_core::providers::Usage { input_tokens: 1800, output_tokens: 1500 };
    let one_call = autopaper_core::pricing::text_cost(ProviderKind::OpenAi, "gpt-6-luna-2026-09-22", &usage);
    assert_eq!(engine.spend_summary().unwrap().spent_microusd, one_call, "what the provider reported is kept");
}

#[tokio::test]
async fn cancel_stops_a_comfyui_job_under_way() {
    let h = Harness::new();
    healthy_services(&h.http);
    h.keywords(&["lighthouse"], &[], &[]);
    h.update(|s| s.image_provider.kind = ProviderKind::ComfyUi);
    let queued = include_str!("fixtures/comfyui/prompt.response.json");
    let id = serde_json::from_str::<serde_json::Value>(queued).unwrap()["prompt_id"].as_str().unwrap().to_string();
    h.http.once("/prompt", 200, queued);
    h.http.always("/history/", 200, "{}");
    h.http.once("/api/jobs/", 200, r#"{"cancelled": true}"#);

    let running = tokio::spawn({
        let engine = h.engine.clone();
        async move { engine.generate(Trigger::Manual, None).await }
    });
    let requested = |part: &str| h.http.requests().iter().any(|request| request.url.contains(part));
    // ComfyUI is painting: its history is polled after a second.
    for _ in 0..100 {
        if requested("/history/") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(requested("/history/"), "{:?}", h.http.requests());
    h.engine.cancel();
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), running).await.expect("promptly").unwrap();
    assert!(matches!(result, Err(AutoPaperError::Cancelled)), "{result:?}");
    for _ in 0..100 {
        if requested("/api/jobs/") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let cancel = format!("http://127.0.0.1:8188/api/jobs/{id}/cancel");
    assert!(h.http.requests().iter().any(|request| request.url == cancel), "the job is stopped in ComfyUI");
}

#[tokio::test]
async fn a_painting_comfyui_cant_run_isnt_retried_on_the_timer() {
    let h = Harness::new();
    healthy_services(&h.http);
    h.keywords(&["lighthouse"], &[], &[]);
    let liked = h.generate().await;
    h.engine.rate(liked.id.clone(), Rating::Liked).unwrap();
    h.clock.advance_days(1.0);
    h.update(|s| s.image_provider.kind = ProviderKind::ComfyUi);
    // A node fails in ComfyUI (the user's run: a model run through another model's graph).
    let queued = include_str!("fixtures/comfyui/prompt.response.json");
    let id = serde_json::from_str::<serde_json::Value>(queued).unwrap()["prompt_id"].as_str().unwrap().to_string();
    let failed = json!({ &id: { "outputs": {}, "status": { "status_str": "error", "completed": false, "messages": [
        ["execution_error", { "prompt_id": &id, "node_id": "8", "node_type": "KSampler",
            "exception_message": "Given normalized_shape=[4096], expected input with shape [*4096], but got input of size[1, 95, 2560]" }],
    ]}}});
    h.http.once("/prompt", 200, queued);
    h.http.once("/history/", 200, failed.to_string());

    let stages = Arc::new(Stages::default());
    let result = h.engine.run_if_due(Some(stages.clone())).await;
    // Not covered by a liked wallpaper (the default fallback), so the host can show the problem and its fix.
    match result {
        Err(AutoPaperError::PaintingFailed { provider: ProviderKind::ComfyUi, model, detail }) => {
            assert_eq!(model, "Z-Image Turbo", "hosts name the model and link to Providers");
            assert!(detail.starts_with("ComfyUI couldn't paint with Z-Image Turbo: the KSampler node (8) failed: Given normalized_shape"), "{detail}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(stages.0.lock().unwrap().last(), Some(&ProgressStage::Generating), "it failed while painting");
    // No retry loop: the slot is filled, so the timer comes back one interval later, not in 10 minutes.
    assert_eq!(h.engine.next_due().unwrap(), Some(h.now() + DAY));
    assert!(h.engine.run_if_due(None).await.unwrap().is_none(), "not due again");
    assert_eq!(h.http.requests().iter().filter(|request| request.url.ends_with("/prompt")).count(), 1);
    let rows = failed_rows(&h);
    assert_eq!(rows.len(), 1);
    assert!(rows[0].1.contains("the KSampler node (8) failed"), "{rows:?}");

    // Asked for by the person, the same failure is the same error (nothing to revisit).
    h.http.once("/prompt", 200, queued);
    h.http.once("/history/", 200, failed.to_string());
    let manual = h.engine.generate(Trigger::Manual, None).await;
    assert!(matches!(manual, Err(AutoPaperError::PaintingFailed { .. })), "{manual:?}");
}

#[tokio::test]
async fn a_model_comfyui_has_no_workflow_for_is_refused_before_an_idea_is_paid_for() {
    let h = Harness::new();
    healthy_services(&h.http);
    h.keywords(&["lighthouse"], &[], &[]);
    // Chosen from Settings before it listed only models with a bundled workflow.
    h.update(|s| {
        s.text_provider.kind = ProviderKind::OpenAi;
        s.image_provider = ProviderSelection {
            kind: ProviderKind::ComfyUi,
            model: "ltx-2.5-22b-distilled-transformer-comfy-int8-convrot.safetensors".into(),
            base_url: None,
        };
    });
    for result in [h.engine.generate(Trigger::Manual, None).await.map(|_| ()), h.engine.run_if_due(None).await.map(|_| ())] {
        assert!(
            matches!(&result, Err(AutoPaperError::PaintingFailed { provider: ProviderKind::ComfyUi, model, .. }) if model == "ltx-2.5-22b-distilled-transformer-comfy-int8-convrot"),
            "{result:?}"
        );
    }
    assert!(work_requests(&h.http).is_empty(), "no idea or painting was requested");
    assert_eq!(h.http.requests().len(), 4, "both services checked for both attempts");
    assert!(failed_rows(&h).is_empty(), "nothing was attempted");
    assert_eq!(h.engine.spend_summary().unwrap().spent_microusd, 0);
    // The scheduled one fills its slot: no retry every 10 minutes until the person picks another model.
    assert_eq!(h.engine.next_due().unwrap(), Some(h.now() + DAY));

    // A model with a workflow is fine (its sizes are its own: Qwen-Image 2.1 paints up to 2752 a side).
    h.update(|s| s.image_provider.model = "qwen_image_2.1_int8_convrot.safetensors".into());
    h.update(|s| s.text_provider.kind = ProviderKind::Demo);
    h.http.once("/prompt", 400, include_str!("fixtures/comfyui/prompt-invalid.response.json"));
    let rejected = h.engine.generate(Trigger::Manual, None).await;
    assert!(
        matches!(&rejected, Err(AutoPaperError::PaintingFailed { model, detail, .. }) if model == "Qwen-Image 2.1" && detail.starts_with("ComfyUI rejected the workflow for Qwen-Image 2.1: ")),
        "{rejected:?}"
    );
    let sent = work_json(&h.http, 0);
    assert_eq!(sent["prompt"]["1"]["inputs"]["unet_name"], "qwen_image_2.1_int8_convrot.safetensors");
    assert_eq!(sent["prompt"]["2"]["inputs"]["clip_name"], "qwen3vl_8b_int8_convrot.safetensors");
}

#[tokio::test]
async fn the_wallpaper_on_the_desktop_keeps_its_renders() {
    let h = Harness::new();
    h.keywords(&["lighthouse"], &[], &[]);
    let display = DisplayTarget { id: "main".into(), width: 64, height: 40 };
    let shown = h.generate().await;
    let other = h.generate().await;
    let render = h.engine.render_for_display(shown.id.clone(), display.clone()).unwrap();
    let other_render = h.engine.render_for_display(other.id.clone(), display.clone()).unwrap();
    h.engine.mark_shown(shown.id.clone()).unwrap();

    // Clearing history (keeping memory) leaves only the desktop's renders.
    h.engine.clear_history(true).unwrap();
    assert_eq!(h.engine.current().unwrap(), None);
    assert!(Path::new(&render).is_file(), "the desktop still points at it");
    assert!(!Path::new(&other_render).exists());
    assert!(!exists(&shown.image_path) && !exists(&shown.thumb_path));
    h.engine.prune().unwrap();
    assert!(Path::new(&render).is_file(), "prune keeps it while it's on the desktop");

    // Once another wallpaper is shown, the next prune deletes them.
    let next = h.generate().await;
    assert!(Path::new(&render).is_file(), "the new one isn't shown yet");
    let next_render = h.engine.render_for_display(next.id.clone(), display.clone()).unwrap();
    h.engine.mark_shown(next.id.clone()).unwrap();
    h.engine.prune().unwrap();
    assert!(!Path::new(&render).exists());
    assert!(Path::new(&next_render).is_file());

    // Deleting the wallpaper on the desktop, or forgetting everything, keeps its renders too.
    h.engine.delete_generation(next.id.clone()).unwrap();
    assert!(Path::new(&next_render).is_file() && !exists(&next.image_path) && !exists(&next.thumb_path));
    h.engine.clear_history(false).unwrap();
    h.engine.prune().unwrap();
    assert!(Path::new(&next_render).is_file(), "still on the desktop");
    let last = h.generate().await;
    h.engine.mark_shown(last.id.clone()).unwrap();
    h.engine.prune().unwrap();
    assert!(!Path::new(&next_render).exists());
    assert_eq!(std::fs::read_dir(h.dir.path().join("renders")).unwrap().count(), 0);
}

#[tokio::test]
async fn invalid_input_says_why() {
    let h = Harness::new();
    let reason = |result: Result<(), AutoPaperError>| match result {
        Err(AutoPaperError::InvalidInput { reason, .. }) => reason,
        other => panic!("{other:?}"),
    };
    assert_eq!(reason(h.engine.add_keyword("  ".into(), KeywordWeight::Must).map(|_| ())), InvalidInputReason::KeywordEmpty);
    let sea = h.engine.add_keyword("sea".into(), KeywordWeight::Must).unwrap();
    h.engine.add_keyword("fog".into(), KeywordWeight::Maybe).unwrap();
    assert_eq!(reason(h.engine.rename_keyword(sea.id.clone(), "Fog".into()).map(|_| ())), InvalidInputReason::DuplicateKeyword);
    let address = |base: &str| {
        let mut settings = h.engine.settings().unwrap();
        settings.text_provider =
            ProviderSelection { kind: ProviderKind::Ollama, model: String::new(), base_url: Some(base.into()) };
        reason(h.engine.update_settings(settings))
    };
    assert_eq!(address("http://ollama.example.com:11434"), InvalidInputReason::AddressNotAllowed);
    assert_eq!(address("ollama:11434"), InvalidInputReason::AddressInvalid);
    let compatible = ProviderSelection { kind: ProviderKind::OpenAiCompatible, model: String::new(), base_url: None };
    let missing = h.engine.test_provider(compatible, autopaper_core::ProviderJob::Concepts).await;
    assert_eq!(reason(missing), InvalidInputReason::AddressMissing);
    let nothing = h.engine.generate(Trigger::EchoRequest, None).await.map(|_| ());
    assert_eq!(reason(nothing), InvalidInputReason::NothingToEcho);
}

#[tokio::test]
async fn a_clock_set_back_waits_at_most_one_interval() {
    let h = Harness::new();
    h.keywords(&["lighthouse"], &[], &[]);
    assert!(h.engine.run_if_due(None).await.unwrap().is_some());

    // The clock ran a month fast and is corrected.
    let corrected = START - 30 * DAY;
    h.clock.set(corrected);
    assert_eq!(h.engine.next_due().unwrap(), Some(corrected + DAY), "one interval from the correction");
    h.clock.set(corrected + DAY / 2);
    assert_eq!(h.engine.next_due().unwrap(), Some(corrected + DAY), "and it stays put");
    assert!(h.engine.run_if_due(None).await.unwrap().is_none());
    h.clock.set(corrected + DAY);
    let made = h.engine.run_if_due(None).await.unwrap().expect("due one interval after the correction");
    assert_eq!(made.generation.created_at, corrected + DAY);
    assert_eq!(h.engine.next_due().unwrap(), Some(corrected + 2 * DAY), "the schedule counts from it");

    // A slot skipped over budget after a correction counts the same way.
    h.update(|s| {
        s.text_provider.kind = ProviderKind::OpenAi;
        s.image_provider.kind = ProviderKind::OpenAi;
        s.monthly_budget_cents = Some(1);
        s.fallback = Fallback::KeepCurrent;
    });
    h.clock.set(corrected + 2 * DAY);
    assert!(matches!(h.engine.run_if_due(None).await, Err(AutoPaperError::BudgetReached { .. })));
    h.clock.set(corrected + DAY + DAY / 2);
    assert_eq!(h.engine.next_due().unwrap(), Some(corrected + DAY + DAY / 2 + DAY));
}

// ── Host callbacks that fail ────────────────────────────────────────────────────────────────

/// A foreign observer that throws: UniFFI turns that into a panic in the core.
struct PanicsAt(ProgressStage);

impl ProgressObserver for PanicsAt {
    fn on_progress(&self, stage: ProgressStage) {
        if stage == self.0 {
            panic!("Callback interface failure: the host's observer threw");
        }
    }
}

#[tokio::test]
async fn a_failing_observer_doesnt_stop_the_wallpaper_or_its_bookkeeping() {
    let h = Harness::new();
    h.keywords(&["lighthouse"], &[], &[]);
    let observer: Arc<dyn ProgressObserver> = Arc::new(PanicsAt(ProgressStage::Downloading));
    let shown = h.engine.run_if_due(Some(observer)).await.unwrap().expect("made anyway");
    assert!(exists(&shown.generation.image_path));
    assert_eq!(h.engine.next_due().unwrap(), Some(h.now() + DAY), "counted as a scheduled wallpaper");
}

/// A foreign secret store that throws (a Credential Manager wrapper failing on "element not found").
struct ThrowingSecrets;

impl SecretStore for ThrowingSecrets {
    fn get(&self, _account: String) -> Option<String> {
        panic!("Callback interface failure: the host's secret store threw")
    }

    fn set(&self, _account: String, _value: String) {}

    fn delete(&self, _account: String) {}
}

#[tokio::test]
async fn a_failing_secret_store_reads_as_a_missing_key() {
    let dir = tempfile::tempdir().unwrap();
    let config = EngineConfig {
        data_dir: dir.path().to_string_lossy().into_owned(),
        model_dir: String::new(),
        locale: "en-US".into(),
        client: "test".into(),
    };
    let deps = Deps {
        clock: Arc::new(FixedClock::at(START)),
        http: Arc::new(StubHttp::new()),
        embedder: Arc::new(HashingEmbedder),
        rng_seed: 1,
    };
    let engine = Engine::open_with(config, Arc::new(ThrowingSecrets), deps).unwrap();
    let mut settings = engine.settings().unwrap();
    settings.text_provider.kind = ProviderKind::OpenAi;
    engine.update_settings(settings).unwrap();
    let error = engine.generate(Trigger::Manual, None).await.unwrap_err();
    assert!(matches!(error, AutoPaperError::MissingKey { provider: ProviderKind::OpenAi }), "{error:?}");
}

#[tokio::test]
async fn a_budget_block_never_waits_for_the_keyring() {
    struct CountingSecrets(std::sync::atomic::AtomicUsize);
    impl SecretStore for CountingSecrets {
        fn get(&self, _account: String) -> Option<String> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            None
        }
        fn set(&self, _account: String, _value: String) {}
        fn delete(&self, _account: String) {}
    }
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(CountingSecrets(std::sync::atomic::AtomicUsize::new(0)));
    let http = Arc::new(StubHttp::new());
    let engine = Engine::open_with(EngineConfig {
        data_dir: dir.path().to_string_lossy().into_owned(), model_dir: String::new(),
        locale: "en-US".into(), client: "test".into(),
    }, secrets.clone(), Deps {
        clock: Arc::new(FixedClock::at(START)), http: http.clone(),
        embedder: Arc::new(HashingEmbedder), rng_seed: 1,
    }).unwrap();
    let mut settings = engine.settings().unwrap();
    settings.text_provider.kind = ProviderKind::OpenAi;
    settings.monthly_budget_cents = Some(0);
    engine.update_settings(settings).unwrap();
    assert!(matches!(engine.generate(Trigger::Manual, None).await, Err(AutoPaperError::BudgetReached { budget_cents: 0 })));
    assert_eq!(secrets.0.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert!(http.requests().is_empty());
    assert_eq!(engine.runs(1, 0).unwrap()[0].status, RunStatus::Blocked);
}

#[test]
fn budget_notice_preserves_the_sub_cent_arithmetic_that_blocks_a_run() {
    let h = Harness::new();
    h.update(|s| {
        s.text_provider.kind = ProviderKind::OpenAi;
        s.text_provider.model = "gpt-6.1-sol".into();
        s.monthly_budget_cents = Some(3);
    });
    h.store().add_spend("2026-01", 10_401, 0).unwrap();
    let status = h.engine.budget_status().unwrap();
    assert_eq!(status.next_cost_microusd, 19_600);
    assert!(status.blocked); // $0.010401 + $0.0196 = $0.030001, just over the $0.03 limit.
    assert!(status.message.contains("$0.010401 estimated spent of a $0.03 limit"));
    assert!(status.message.contains("next wallpaper is estimated at $0.0196"));
}

/// Wakes `1` when the generation reaches stage `0`.
struct NotifyAt(ProgressStage, Arc<tokio::sync::Notify>);

impl ProgressObserver for NotifyAt {
    fn on_progress(&self, stage: ProgressStage) {
        if stage == self.0 {
            self.1.notify_one();
        }
    }
}

#[tokio::test]
async fn a_call_the_host_drops_still_records_what_it_spent() {
    let h = Harness::new();
    healthy_services(&h.http);
    h.keywords(&["lighthouse"], &[], &[]);
    h.update(|s| s.text_provider.kind = ProviderKind::OpenAi);
    h.http.once("/v1/responses", 200, responses(&[harbour()]));
    let painting = Arc::new(tokio::sync::Notify::new());
    let observer: Arc<dyn ProgressObserver> = Arc::new(NotifyAt(ProgressStage::Generating, painting.clone()));
    // The host's task is cancelled (a Swift Task, a C# token) while the image is being painted: UniFFI
    // drops the future.
    tokio::select! {
        biased;
        () = painting.notified() => {}
        result = h.engine.generate(Trigger::Manual, Some(observer)) => panic!("finished first: {result:?}"),
    }
    let usage = autopaper_core::providers::Usage { input_tokens: 1800, output_tokens: 1500 };
    let one_call = autopaper_core::pricing::text_cost(ProviderKind::OpenAi, "gpt-6-luna-2026-09-22", &usage);
    assert_eq!(h.engine.spend_summary().unwrap().spent_microusd, one_call, "the paid text call counts");
    let run = h.engine.run(h.engine.runs(1, 0).unwrap()[0].id.clone()).unwrap();
    assert_eq!(run.status, RunStatus::Interrupted);
    assert_eq!(run.cost_microusd, one_call);
    assert_eq!(run.finished_at, Some(h.now()));
    assert!(run.detail.contains("provider may still have billed"));
}

// ── Storage: prune and files ────────────────────────────────────────────────────────────────

/// A sparse file nothing may prune, standing in for years of liked 4K originals.
fn fill_storage(h: &Harness) {
    let filler = h.dir.path().join("thumbs").join("filler.bin");
    std::fs::File::create(&filler).unwrap().set_len(300 * 1024 * 1024).unwrap();
    h.update(|s| s.storage_limit_mb = 256);
}

#[tokio::test]
async fn a_new_wallpaper_is_never_pruned_before_the_host_can_show_it() {
    let h = Harness::new();
    h.keywords(&["lighthouse"], &[], &[]);
    let liked = h.generate().await;
    h.engine.rate(liked.id.clone(), Rating::Liked).unwrap();
    h.engine.mark_shown(liked.id.clone()).unwrap();
    fill_storage(&h);

    h.clock.advance_days(1.0);
    let fresh = h.generate().await;
    assert!(exists(&fresh.image_path), "the wallpaper just made was pruned");
    assert_eq!(h.engine.generation(fresh.id.clone()).unwrap().image_path, fresh.image_path);
    let display = DisplayTarget { id: "main".into(), width: 64, height: 36 };
    h.engine.render_for_display(fresh.id.clone(), display).expect("the host can render it");

    // Shown and then replaced, it is prunable like any other.
    h.engine.mark_shown(fresh.id.clone()).unwrap();
    h.clock.advance_days(1.0);
    let next = h.generate().await;
    assert!(exists(&fresh.image_path) && exists(&next.image_path), "current, and just made");
    h.engine.mark_shown(next.id.clone()).unwrap();
    h.engine.prune().unwrap();
    assert!(!exists(&fresh.image_path) && exists(&next.image_path) && exists(&liked.image_path));
}

#[tokio::test]
async fn prune_keeps_track_of_an_image_it_couldnt_delete() {
    let h = Harness::new();
    h.keywords(&["lighthouse"], &[], &[]);
    let old = h.generate().await;
    h.clock.advance_days(1.0);
    let newer = h.generate().await;
    h.engine.mark_shown(newer.id.clone()).unwrap();
    // Stand-in for a file another app holds open (Windows refuses to delete it): a directory in its place
    // can't be removed as a file.
    let path = old.image_path.clone().unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    std::fs::write(Path::new(&path).join("held"), b"x").unwrap();
    fill_storage(&h);

    h.engine.prune().unwrap();
    assert_eq!(h.engine.generation(old.id.clone()).unwrap().image_path.as_ref(), Some(&path), "still tracked");
    let error = h.engine.delete_generation(old.id.clone()).unwrap_err();
    assert!(matches!(error, AutoPaperError::Storage { .. }), "{error:?}");
    let kept = h.engine.generation(old.id.clone()).expect("the row stays while its file does");
    assert_eq!((kept.image_path.as_ref(), kept.thumb_path.as_ref()), (Some(&path), None), "the thumbnail did go");

    // Once the file can go, it goes.
    std::fs::remove_dir_all(&path).unwrap();
    h.engine.delete_generation(old.id.clone()).unwrap();
    assert!(matches!(h.engine.generation(old.id.clone()), Err(AutoPaperError::NotFound)));
}

#[tokio::test]
async fn files_no_wallpaper_owns_are_cleaned_up() {
    let h = Harness::new();
    h.keywords(&["lighthouse"], &[], &[]);
    let kept = h.generate().await;
    // What a call dropped between writing its files and storing its row leaves behind (two hours ago).
    let id = "019a0000-0000-7000-8000-000000000000";
    let orphan_image = h.dir.path().join("images").join("2026").join(format!("{id}.png"));
    let orphan_thumb = h.dir.path().join("thumbs").join(format!("{id}.jpg"));
    let other = h.dir.path().join("thumbs").join("notes.txt");
    let fresh = h.dir.path().join("thumbs").join("019a0000-0000-7000-8000-000000000001.jpg");
    let two_hours_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 3600);
    for path in [&orphan_image, &orphan_thumb, &other, &fresh] {
        std::fs::write(path, b"x").unwrap();
        if path != &fresh {
            std::fs::File::options().write(true).open(path).unwrap().set_modified(two_hours_ago).unwrap();
        }
    }
    h.engine.prune().unwrap();
    assert!(!orphan_image.exists() && !orphan_thumb.exists(), "orphans are removed");
    assert!(other.exists(), "files the engine didn't name are left alone");
    assert!(fresh.exists(), "a new file may belong to another process's wallpaper still being stored");
    assert!(exists(&kept.image_path) && exists(&kept.thumb_path));
}

// ── Taste and memory ────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn learned_taste_keeps_its_words_after_history_is_cleared() {
    let h = Harness::new();
    h.keywords(&["lighthouse"], &[], &[]);
    for _ in 0..6 {
        let made = h.generate().await;
        h.engine.rate(made.id, Rating::Liked).unwrap();
        h.clock.advance_days(1.0);
    }
    let before = h.engine.taste_summary().unwrap();
    assert!(before.liked.len() > 1, "{before:?}");
    h.engine.clear_history(true).unwrap();
    assert_eq!(h.engine.taste_summary().unwrap().liked, before.liked, "in words, not stem keys");
}

#[tokio::test]
async fn memory_status_says_when_memory_runs_on_the_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let config = EngineConfig {
        data_dir: dir.path().join("data").to_string_lossy().into_owned(),
        model_dir: dir.path().join("no-model-here").to_string_lossy().into_owned(),
        locale: "en-US".into(),
        client: "test".into(),
    };
    let engine = Engine::open(config, Arc::new(StubSecrets::with(&[]))).unwrap();
    let status = engine.memory_status();
    assert!(status.reduced, "{status:?}");
    assert_eq!(status.embedding_model, HashingEmbedder::MODEL_ID);
    assert!(status.problem.is_some(), "{status:?}");

    let h = Harness::with(Arc::new(OldModel), 3);
    let status = h.engine.memory_status();
    assert!(!status.reduced && status.problem.is_none(), "{status:?}");
    assert_eq!(status.embedding_model, "old-model");
}

// ── Moods ───────────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn moods_switch_keywords_and_surprise_without_making_anything() {
    let h = Harness::new();
    h.keywords(&["lighthouse"], &["fog"], &[]);
    h.update(|s| s.surprise = 0.2);
    let first = h.engine.active_mood().unwrap();
    assert_eq!((first.name.as_str(), first.active, first.keywords.len()), ("My mood", true, 2));

    // A second mood, edited while it isn't the one in use.
    let beach = h.engine.create_mood("Rainy beach".into(), None).unwrap();
    assert!(!beach.active);
    h.engine.add_mood_keyword(beach.id.clone(), "beach".into(), KeywordWeight::Must).unwrap();
    h.engine.add_mood_keyword(beach.id.clone(), "rain".into(), KeywordWeight::Maybe).unwrap();
    h.engine.set_mood_surprise(beach.id.clone(), 7.0).unwrap();
    assert_eq!(h.engine.keywords().unwrap().len(), 2, "still the first mood's");
    assert!((h.engine.settings().unwrap().surprise - 0.2).abs() < 1e-6);

    // Switching makes nothing by itself.
    h.engine.set_active_mood(beach.id.clone()).unwrap();
    assert!(h.engine.history(HistoryFilter::All, 10, 0).unwrap().is_empty());
    assert_eq!(h.engine.keywords().unwrap().iter().map(|k| k.text.as_str()).collect::<Vec<_>>(), ["beach", "rain"]);
    assert_eq!(h.engine.settings().unwrap().surprise, 1.0, "clamped, and the active mood's");
    let at_beach = h.generate().await;
    assert_eq!((at_beach.mood_id.as_deref(), at_beach.mood_name.as_deref()), (Some(beach.id.as_str()), Some("Rainy beach")));
    assert!(autopaper_core::text::mentions(&at_beach.concept.prompt, "beach"));
    assert_eq!(at_beach.surprise, 1.0);
    assert_eq!(at_beach.keywords.iter().map(|k| k.text.as_str()).collect::<Vec<_>>(), ["beach", "rain"]);

    h.engine.set_active_mood(first.id.clone()).unwrap();
    let at_home = h.generate().await;
    assert_eq!(at_home.mood_id.as_deref(), Some(first.id.as_str()));
    assert!(autopaper_core::text::mentions(&at_home.concept.prompt, "lighthouse"));
    let ids = |list: Vec<Generation>| list.into_iter().map(|g| g.id).collect::<Vec<_>>();
    assert_eq!(ids(h.engine.history_by_mood(HistoryFilter::All, Some(beach.id.clone()), 10, 0).unwrap()), std::slice::from_ref(&at_beach.id));
    assert_eq!(ids(h.engine.history_by_mood(HistoryFilter::All, None, 10, 0).unwrap()), [at_home.id.clone(), at_beach.id.clone()]);

    // Duplicate, rename, reorder; deleting the active mood moves to the next one.
    let copy = h.engine.create_mood("Rainy beach 2".into(), Some(beach.id.clone())).unwrap();
    assert_eq!(copy.keywords.len(), 2);
    assert!(matches!(
        h.engine.rename_mood(copy.id.clone(), "rainy BEACH".into()),
        Err(AutoPaperError::InvalidInput { reason: InvalidInputReason::DuplicateMoodName, .. })
    ));
    h.engine.rename_mood(copy.id.clone(), "Stormy beach".into()).unwrap();
    h.engine.move_mood(copy.id.clone(), 0).unwrap();
    let names = |engine: &Engine| engine.moods().unwrap().into_iter().map(|m| m.name).collect::<Vec<_>>();
    assert_eq!(names(&h.engine), ["Stormy beach", "My mood", "Rainy beach"]);
    h.engine.delete_mood(first.id.clone()).unwrap();
    assert_eq!(h.engine.active_mood().unwrap().id, beach.id, "the next in the list");
    assert_eq!(h.engine.generation(at_home.id.clone()).unwrap().mood_name, None, "its wallpapers stay, unnamed");
    h.engine.delete_mood(copy.id.clone()).unwrap();
    assert!(matches!(
        h.engine.delete_mood(beach.id.clone()),
        Err(AutoPaperError::InvalidInput { reason: InvalidInputReason::LastMood, .. })
    ));
    assert!(matches!(h.engine.set_active_mood("nope".into()), Err(AutoPaperError::NotFound)));
}

#[tokio::test]
async fn the_painting_model_chosen_in_settings_is_sent_and_recorded() {
    // Settings → Providers saves a painting model with `update_settings`; the next wallpaper asks that model and
    // records it (what the apps' "Painted by …" line shows), and the blank choice records the default by name.
    let h = Harness::new();
    healthy_services(&h.http);
    h.keywords(&["lighthouse"], &[], &[]);
    h.update(|s| s.image_provider = ProviderSelection { kind: ProviderKind::OpenAi, model: "gpt-image-2.5-sunburst".into(), base_url: None });
    assert_eq!(h.engine.settings().unwrap().image_provider.model, "gpt-image-2.5-sunburst", "saved");
    h.http.always("/v1/images/generations", 200, include_str!("fixtures/openai/images_generations.json"));

    let chosen = h.engine.generate(Trigger::Manual, None).await.unwrap();
    assert_eq!(work_json(&h.http, 0)["model"], "gpt-image-2.5-sunburst", "asked of OpenAI");
    assert_eq!((chosen.image_provider, chosen.image_model.as_str()), (ProviderKind::OpenAi, "gpt-image-2.5-sunburst"));
    assert_eq!(h.engine.generation(chosen.id.clone()).unwrap().image_model, "gpt-image-2.5-sunburst", "as stored");

    h.update(|s| s.image_provider.model = String::new());
    let default = h.engine.generate(Trigger::Manual, None).await.unwrap();
    assert_eq!(work_json(&h.http, 1)["model"], "gpt-image-2.5-flare");
    assert_eq!(default.image_model, "gpt-image-2.5-flare", "the default, by name");
}

#[tokio::test]
async fn mood_stats_sum_up_every_mood() {
    let h = Harness::new();
    h.keywords(&["lighthouse"], &[], &[]);
    let home = h.engine.active_mood().unwrap();
    let beach = h.engine.create_mood("Rainy beach".into(), None).unwrap();
    let mut made = Vec::new();
    for _ in 0..4 {
        made.push(h.generate().await);
        h.clock.advance_days(1.0);
    }
    made.push(h.engine.make_echo(made[0].id.clone(), None).await.unwrap());
    h.engine.rate(made[1].id.clone(), Rating::Liked).unwrap();
    h.engine.rate(made[4].id.clone(), Rating::Disliked).unwrap();

    let stats = h.engine.mood_stats().unwrap();
    assert_eq!(stats.iter().map(|s| s.mood_id.as_str()).collect::<Vec<_>>(), [home.id.as_str(), beach.id.as_str()]);
    let at_home = &stats[0];
    assert_eq!((at_home.wallpapers, at_home.liked, at_home.disliked, at_home.echoes), (5, 1, 1, 1));
    assert_eq!(at_home.last_made_at, Some(made[4].created_at));
    let newest: Vec<&str> = made.iter().rev().take(MoodStats::LATEST).map(|g| g.id.as_str()).collect();
    assert_eq!(at_home.latest.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(), newest);
    assert_eq!(at_home.latest[0].rating, Rating::Disliked, "as rated now");
    let empty = &stats[1];
    assert_eq!((empty.wallpapers, empty.liked, empty.disliked, empty.echoes, empty.last_made_at), (0, 0, 0, 0, None));
    assert!(empty.latest.is_empty());

    // One a day for four days, then the echo on the fifth; a sixth day with none isn't listed.
    let day = 86_400;
    let bounds: Vec<i64> = (0..=6).map(|i| made[0].created_at + i * day).collect();
    let activity = h.engine.activity(bounds.clone()).unwrap();
    assert_eq!(
        activity.iter().map(|c| (c.day_start, c.mood_id.as_deref(), c.count)).collect::<Vec<_>>(),
        bounds[..5].iter().map(|start| (*start, Some(home.id.as_str()), 1)).collect::<Vec<_>>()
    );
    assert!(matches!(
        h.engine.activity(vec![bounds[1], bounds[0]]),
        Err(AutoPaperError::InvalidInput { reason: InvalidInputReason::Other, .. })
    ));
}

#[tokio::test]
async fn echoes_prefer_originals_made_under_the_active_mood() {
    // One original per mood, both past the quiet period; the active mood's is 3× as likely. Fixed seeds, so the
    // count is the same every run.
    let mut in_mood = 0;
    let runs = 24;
    for seed in 0..runs {
        let h = Harness::with(Arc::new(HashingEmbedder), seed);
        h.keywords(&["lighthouse"], &[], &[]);
        let rain = h.engine.active_mood().unwrap();
        let mine = h.generate().await;
        let snow = h.engine.create_mood("Snow".into(), None).unwrap();
        h.engine.set_active_mood(snow.id.clone()).unwrap();
        h.keywords(&["glacier"], &[], &[]);
        let theirs = h.generate().await;
        h.engine.set_active_mood(rain.id.clone()).unwrap();
        h.clock.advance_days(200.0);
        let echo = h.engine.generate(Trigger::EchoRequest, None).await.unwrap();
        assert_eq!(echo.mood_id.as_deref(), Some(rain.id.as_str()), "an echo is made under the active mood");
        match echo.echo_of.as_deref() {
            Some(id) if id == mine.id => in_mood += 1,
            Some(id) if id == theirs.id => {}
            other => panic!("{other:?}"),
        }
    }
    assert!(in_mood >= 14 && in_mood < runs, "{in_mood} of {runs}: about three in four, and the other mood's still come back");
}

// ── Performance history and progress ────────────────────────────────────────────────────────

/// Records every `ProgressDetail`.
#[derive(Default)]
struct Details(Mutex<Vec<ProgressDetail>>);

impl ProgressDetailObserver for Details {
    fn on_progress_detail(&self, detail: ProgressDetail) {
        self.0.lock().unwrap().push(detail);
    }
}

fn demo() -> ProviderSelection {
    ProviderSelection { kind: ProviderKind::Demo, model: String::new(), base_url: None }
}

#[tokio::test]
async fn estimates_appear_after_a_run_and_follow_the_size() {
    let h = Harness::new();
    h.keywords(&["lighthouse"], &[], &[]);
    h.engine.set_demo_delay(std::time::Duration::from_millis(800));
    assert_eq!(h.engine.estimate(demo(), ProviderJob::Images, 0, 0).unwrap(), None, "nothing recorded: no number");
    assert_eq!(h.engine.estimate(demo(), ProviderJob::Concepts, 0, 0).unwrap(), None);
    h.generate().await;
    // Demo's slow mode: a quarter writing (0.2 s), the rest painting (0.6 s); whole seconds, at least 1.
    assert_eq!(h.engine.estimate(demo(), ProviderJob::Concepts, 0, 0).unwrap(), Some(1));
    assert_eq!(h.engine.estimate(demo(), ProviderJob::Images, 0, 0).unwrap(), Some(1), "at the size it would ask for");
    let at_size = h.engine.estimate(demo(), ProviderJob::Images, DISPLAY.0, DISPLAY.1).unwrap();
    assert_eq!(at_size, Some(1));
    // Forty times the pixels: forty times the time.
    let bigger = h.engine.estimate(demo(), ProviderJob::Images, DISPLAY.0 * 8, DISPLAY.1 * 5).unwrap().unwrap();
    assert!((24..=30).contains(&bigger), "{bigger}");
    // Another painter, or the same one on another server, has no history yet.
    let comfy = ProviderSelection { kind: ProviderKind::ComfyUi, model: String::new(), base_url: None };
    assert_eq!(h.engine.estimate(comfy.clone(), ProviderJob::Images, 0, 0).unwrap(), None);
    let store = h.store();
    let recorded = store.timings(ProviderJob::Images, ProviderKind::Demo, "", "demo").unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!((recorded[0].width, recorded[0].height, recorded[0].steps), (DISPLAY.0, DISPLAY.1, Some(8)));
    assert!((0.55..2.0).contains(&recorded[0].seconds), "{:?}", recorded[0]);
    assert_eq!(recorded[0].finished_at, START);
}

#[tokio::test]
async fn painting_reports_a_fraction_and_time_left_to_the_detail_observer() {
    let h = Harness::new();
    h.keywords(&["lighthouse"], &[], &[]);
    h.engine.set_demo_delay(std::time::Duration::from_millis(1600));
    let details = Arc::new(Details::default());
    h.engine.set_progress_detail_observer(Some(details.clone()));
    let stages = Arc::new(Stages::default());
    h.engine.generate(Trigger::Manual, Some(stages.clone())).await.unwrap();
    assert_eq!(stages.0.lock().unwrap().len(), 7, "the per-call observer still hears every stage");
    let first = std::mem::take(&mut *details.0.lock().unwrap());
    let painting: Vec<&ProgressDetail> = first.iter().filter(|d| d.stage == ProgressStage::Generating).collect();
    assert_eq!(painting[0].fraction, None, "no estimate yet: indeterminate until a step");
    let fractions: Vec<f32> = painting.iter().filter_map(|d| d.fraction).collect();
    assert!(fractions.len() >= 6, "about one per step: {fractions:?}");
    assert!(fractions.windows(2).all(|pair| pair[0] <= pair[1]), "never backwards: {fractions:?}");
    assert!(*fractions.last().unwrap() >= 0.85 && *fractions.last().unwrap() <= 0.99, "{fractions:?}");
    assert!(painting.iter().any(|d| d.seconds_left.is_some()), "paced by the steps once two have come");
    assert_eq!(
        first.last(),
        Some(&ProgressDetail { stage: ProgressStage::Done, fraction: Some(1.0), seconds_left: Some(0) })
    );
    assert!(first.iter().any(|d| d.stage == ProgressStage::Composing && d.fraction.is_none()));

    // The next one knows how long painting takes here: about 1.2 s left at the start.
    h.engine.generate(Trigger::Manual, None).await.unwrap();
    let second = std::mem::take(&mut *details.0.lock().unwrap());
    let start = second.iter().find(|d| d.stage == ProgressStage::Generating && d.fraction.is_some()).unwrap();
    assert!(start.fraction.unwrap() < 0.2 && start.seconds_left == Some(2), "{start:?}");

    // Without an observer nothing is reported (and nothing breaks).
    h.engine.set_progress_detail_observer(None);
    h.engine.generate(Trigger::Manual, None).await.unwrap();
    assert!(details.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_scheduled_wallpaper_starts_early_by_its_estimate_and_fills_its_slot() {
    const HOUR: i64 = 3600;
    let h = Harness::new();
    h.keywords(&["lighthouse"], &[], &[]);
    h.update(|s| s.cadence = Cadence::Every6Hours);
    assert_eq!(h.engine.next_start().unwrap(), h.engine.next_due().unwrap(), "nothing recorded: start when due");
    h.engine.set_demo_delay(std::time::Duration::from_secs(2));
    let first = h.engine.run_if_due(None).await.unwrap().expect("made");
    let due = START + 6 * HOUR;
    assert_eq!(h.engine.next_due().unwrap(), Some(due));
    let start = h.engine.next_start().unwrap().unwrap();
    assert!((1..=3).contains(&(due - start)), "about the 2 s a wallpaper takes: {}", due - start);

    h.clock.set(start - 1);
    assert!(h.engine.run_if_due(None).await.unwrap().is_none());
    h.clock.set(start);
    let early = h.engine.run_if_due(None).await.unwrap().expect("started early");
    assert_eq!(early.generation.trigger, Trigger::Scheduled);
    assert_ne!(early.generation.id, first.generation.id);
    assert_eq!(h.engine.next_due().unwrap(), Some(due + 6 * HOUR), "it filled the slot due at {due}, not its start");

    // Paused or manual: neither.
    h.update(|s| s.cadence = Cadence::Manual);
    assert_eq!(h.engine.next_start().unwrap(), None);
}

#[tokio::test]
async fn comfyuis_steps_reach_the_detail_observer() {
    let h = Harness::new();
    healthy_services(&h.http);
    h.keywords(&["lighthouse"], &[], &[]);
    h.update(|s| s.image_provider.kind = ProviderKind::ComfyUi);
    let queued = include_str!("fixtures/comfyui/prompt.response.json");
    let id = serde_json::from_str::<serde_json::Value>(queued).unwrap()["prompt_id"].as_str().unwrap().to_string();
    let event = |kind: &str, data: serde_json::Value| {
        let mut data = data;
        data["prompt_id"] = json!(id);
        SocketStep::Text(json!({ "type": kind, "data": data }).to_string())
    };
    let wait = || SocketStep::Wait(std::time::Duration::from_millis(150));
    let mut script = vec![event("execution_start", json!({})), event("executing", json!({ "node": "8" }))];
    for value in 1..=8 {
        script.push(wait());
        script.push(event("progress", json!({ "value": value, "max": 8, "node": "8" })));
    }
    script.push(event("executing", json!({ "node": null })));
    h.http.socket("/ws", script);
    h.http.once("/prompt", 200, queued);
    // Still painting at the first poll (1 s); the socket's "done" (1.2 s) brings the next one forward.
    h.http.once("/history/", 200, "{}");
    h.http.once("/history/", 200, include_str!("fixtures/comfyui/history.response.json"));
    h.http.once_response(
        "/view",
        HttpResponse {
            status: 200,
            headers: vec![("content-type".into(), "image/png".into())],
            body: include_bytes!("fixtures/openai_compat/tiny.png").to_vec(),
        },
    );
    let details = Arc::new(Details::default());
    h.engine.set_progress_detail_observer(Some(details.clone()));
    let made = h.generate().await;
    let fractions: Vec<f32> =
        details.0.lock().unwrap().iter().filter(|d| d.stage == ProgressStage::Generating).filter_map(|d| d.fraction).collect();
    assert_eq!(fractions.first(), Some(&0.125), "{fractions:?}");
    assert!(fractions.contains(&0.5) && fractions.windows(2).all(|pair| pair[0] < pair[1]), "{fractions:?}");
    assert!(fractions.last().is_some_and(|last| *last >= 0.875), "the last steps, until the picture arrived: {fractions:?}");
    // Its timing is filed under the server and the model, with the workflow's steps.
    let timings = h.store().timings(ProviderJob::Images, ProviderKind::ComfyUi, "http://127.0.0.1:8188", &made.image_model).unwrap();
    assert_eq!(timings.len(), 1);
    assert_eq!((timings[0].width, timings[0].height, timings[0].steps), (144, 64, Some(8)), "the size asked for (16s)");
    let selection = ProviderSelection { kind: ProviderKind::ComfyUi, model: String::new(), base_url: None };
    assert!(h.engine.estimate(selection, ProviderJob::Images, 0, 0).unwrap().is_some(), "Settings can say how long it takes");
}

// Console uses the real engine seams, so retries and blocks are tested before the native UI reads them.
#[tokio::test]
async fn console_never_retains_an_opaque_key_echoed_in_a_provider_error() {
    let h = Harness::new();
    healthy_services(&h.http);
    h.keywords(&["lighthouse"], &[], &[]);
    let secrets = Arc::new(StubSecrets::with(&[("openai.api_key", "opaque-provider-key")]));
    let engine = Engine::open_with(EngineConfig {
        data_dir: h.dir.path().to_string_lossy().into_owned(), model_dir: String::new(),
        locale: "en-US".into(), client: "test".into(),
    }, secrets, Deps {
        clock: h.clock.clone(), http: h.http.clone(), embedder: Arc::new(HashingEmbedder), rng_seed: 1,
    }).unwrap();
    let mut settings = engine.settings().unwrap();
    settings.text_provider.kind = ProviderKind::OpenAi;
    engine.update_settings(settings).unwrap();
    h.http.once("/v1/responses", 401, br#"{"error":{"message":"Invalid opaque-provider-key"}}"#.to_vec());
    assert!(matches!(engine.generate(Trigger::Manual, None).await, Err(AutoPaperError::InvalidKey { .. })));
    let run = engine.runs(1, 0).unwrap().remove(0);
    let report = engine.run_report(run.id).unwrap();
    assert!(report.contains("[redacted]") && !report.contains("opaque-provider-key"));
}

#[tokio::test]
async fn console_explains_a_failed_model_lookup_before_any_prompt_is_sent() {
    let h = Harness::new();
    h.keywords(&["lighthouse"], &[], &[]);
    h.update(|s| s.text_provider.kind = ProviderKind::Ollama);
    h.http.always("/api/tags", 503, br#"{"error":"Models are unavailable"}"#.to_vec());
    assert!(h.engine.generate(Trigger::Manual, None).await.is_err());
    let run = h.engine.run(h.engine.runs(1, 0).unwrap()[0].id.clone()).unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    assert!(!run.detail.contains("No network request"));
    assert!(run.events.iter().any(|event| event.kind == "request" && event.detail.contains("GET http://127.0.0.1:11434/api/tags")));
    assert!(run.events.iter().any(|event| event.kind == "response" && event.detail.contains("HTTP 503")));
}

#[tokio::test]
async fn console_keeps_full_requests_responses_and_rejection_reasons_for_each_retry() {
    let h = Harness::new();
    healthy_services(&h.http);
    h.keywords(&["lighthouse"], &[], &[]);
    h.update(|s| s.text_provider.kind = ProviderKind::OpenAi);
    h.http.always("/v1/responses", 200, responses(&[no_lighthouse()]));
    assert!(matches!(h.engine.generate(Trigger::Manual, None).await,
        Err(AutoPaperError::KeywordNotFollowed { .. })));
    let summary = h.engine.runs(10, 0).unwrap().remove(0);
    assert!(summary.events.is_empty(), "lists return summaries; selecting loads the full trace");
    let run = h.engine.run(summary.id.clone()).unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    let requests: Vec<_> = run.events.iter().filter(|event| event.kind == "request" && event.detail.contains("responses")).collect();
    assert_eq!(requests.len(), 3);
    assert_eq!(run.events.iter().filter(|event| event.kind == "response").count(), 4);
    assert!(requests[0].detail.contains("lighthouse") && requests[0].detail.contains("responses"));
    assert!(run.events.iter().any(|event| event.kind == "evaluation" && event.detail.contains("lighthouse")));
    assert!(run.events.iter().any(|event| event.kind == "instructions" && event.detail.contains("left out")));
    let report = h.engine.run_report(run.id).unwrap();
    assert!(!report.contains("sk-test-0123456789abcdefghij") && !report.contains("Authorization"));
    assert!(h.engine.history(HistoryFilter::All, 10, 0).unwrap().is_empty());
    let models = h.engine.console_statistics().unwrap().models;
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].model, "gpt-6-luna-2026-09-22", "the response model, even when the request used an alias");
    assert_eq!(models[0].calls, 3, "each real completed retry is measured");
}

#[tokio::test]
async fn console_success_cancellation_and_missing_key_have_honest_distinct_outcomes() {
    let h = Harness::new();
    h.keywords(&["lighthouse"], &[], &[]);
    let made = h.generate().await;
    let record = h.engine.runs(1, 0).unwrap().remove(0);
    assert_eq!(record.status, RunStatus::Succeeded);
    assert_eq!(record.generation_id.as_deref(), Some(made.id.as_str()));
    let record = h.engine.run(record.id).unwrap();
    assert!(record.events.iter().any(|event| event.kind == "parameters" && event.detail.contains(&made.concept.prompt)));
    assert!(record.events.iter().all(|event| event.kind != "request"), "Demo has no network requests");
    let stats = h.engine.console_statistics().unwrap();
    assert_eq!(stats.total, 1);
    assert_eq!(stats.success_rate, Some(1.0));
    assert!(stats.models.iter().any(|model| model.job == ProviderJob::Concepts && model.calls > 0));
    assert!(stats.models.iter().any(|model| model.job == ProviderJob::Images && model.calls > 0));
    h.update(|s| s.text_provider.kind = ProviderKind::Google);
    assert!(matches!(h.engine.generate(Trigger::Manual, None).await, Err(AutoPaperError::MissingKey { .. })));
    let blocked = h.engine.runs(1, 0).unwrap().remove(0);
    assert_eq!(blocked.status, RunStatus::Blocked);
    assert!(blocked.detail.contains("No network request"));
    h.update(|s| s.text_provider.kind = ProviderKind::Demo);
    let canceller = Arc::new(CancelWhileComposing(Mutex::new(Some(h.engine.clone()))));
    assert!(matches!(h.engine.generate(Trigger::Manual, Some(canceller)).await, Err(AutoPaperError::Cancelled)));
    assert_eq!(h.engine.runs(1, 0).unwrap()[0].status, RunStatus::Cancelled);
    h.engine.clear_runs().unwrap();
    assert!(h.engine.runs(10, 0).unwrap().is_empty());
    assert_eq!(h.engine.console_statistics().unwrap().total, 0);
    assert!(h.engine.console_statistics().unwrap().models.is_empty());
    assert!(h.engine.generation(made.id).is_ok(), "clearing diagnostics preserves wallpaper memory");
}

#[tokio::test]
async fn console_recovers_interrupted_runs_and_enforces_count_age_and_paging_limits() {
    let mut h = Harness::new();
    h.generate().await;
    let mut run = h.engine.run(h.engine.runs(1, 0).unwrap()[0].id.clone()).unwrap();
    run.status = RunStatus::Running;
    run.finished_at = None;
    h.store().save_run(&run).unwrap();
    h.reopen(Arc::new(HashingEmbedder));
    assert_eq!(h.engine.run(run.id.clone()).unwrap().status, RunStatus::Interrupted);
    assert!(h.engine.run(run.id.clone()).unwrap().detail.contains("may be unknown"));
    run.status = RunStatus::Succeeded;
    for n in 0..205 {
        run.id = format!("retained-{n}");
        run.started_at = h.now() + n;
        h.store().save_run(&run).unwrap();
    }
    assert_eq!(h.engine.runs(1000, 0).unwrap().len(), 100);
    assert_eq!(h.engine.runs(100, 100).unwrap().len(), 100);
    assert!(h.engine.runs(100, 200).unwrap().is_empty());
    assert_eq!(h.engine.console_statistics().unwrap().total, 200, "statistics include every retained run, not just a visible page");
    h.clock.advance_days(31.0);
    assert!(h.engine.runs(100, 0).unwrap().is_empty());
    assert_eq!(h.engine.console_statistics().unwrap().total, 0);
}


// ── Service preflight and selected-mood fallback ────────────────────────────────────────────

fn local_services(h: &Harness) {
    h.update(|settings| {
        settings.text_provider = ProviderSelection { kind: ProviderKind::Ollama, model: "m".into(), base_url: None };
        settings.image_provider = ProviderSelection { kind: ProviderKind::ComfyUi, model: String::new(), base_url: None };
        settings.fallback = Fallback::KeepCurrent;
    });
}

#[tokio::test]
async fn service_preflight_checks_both_roles_and_revisits_latest_selected_mood_without_spending() {
    for (writing_down, painting_down) in [(true, false), (false, true), (true, true)] {
        let h = Harness::new();
        h.keywords(&["lighthouse"], &[], &[]);
        let mood = h.engine.active_mood().unwrap();
        let oldest = h.generate().await;
        h.clock.set(h.now() + 1);
        let latest = h.generate().await;
        let other = h.engine.create_mood("Other".into(), Some(mood.id.clone())).unwrap();
        h.engine.set_active_mood(other.id).unwrap();
        h.clock.set(h.now() + 1);
        let elsewhere = h.generate().await;
        h.engine.mark_shown(elsewhere.id.clone()).unwrap();
        h.engine.set_active_mood(mood.id.clone()).unwrap();
        local_services(&h);
        if !writing_down { h.http.always("/api/tags", 200, br#"{"models":[{"name":"m"}]}"#.to_vec()); }
        if !painting_down { h.http.always("/object_info", 200, b"{}".to_vec()); }
        let stages = Arc::new(Stages::default());
        let shown = h.engine.generate_or_revisit(Trigger::Manual, Some(stages.clone())).await.unwrap();
        assert_eq!(shown.revisit, Some(RevisitReason::ServicesUnavailable));
        assert_eq!(shown.generation.id, latest.id);
        assert_ne!(shown.generation.id, oldest.id);
        assert_eq!(*stages.0.lock().unwrap(), [ProgressStage::CheckingServices]);
        let requests = h.http.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests.iter().all(|request| request.body.is_none()));
        assert!(requests.iter().any(|request| request.url.ends_with("/api/tags")));
        assert!(requests.iter().any(|request| request.url.ends_with("/object_info")));
        let run = h.engine.run(h.engine.runs(1, 0).unwrap()[0].id.clone()).unwrap();
        assert_eq!((run.status, run.cost_microusd, run.generation_id), (RunStatus::Failed, 0, None));
        let checks: Vec<_> = run.events.iter().filter(|event| event.kind == "service_check").collect();
        assert_eq!(checks.len(), 2);
        assert!(checks.iter().all(|event| event.stage == "Checking services"));
        assert!(checks.iter().any(|event| event.detail.starts_with("Writing:")));
        assert!(checks.iter().any(|event| event.detail.starts_with("Painting:")));
        assert!(run.events.iter().any(|event| event.kind == "fallback" && event.detail.contains(&latest.concept.title)));
        assert_eq!(h.engine.history(HistoryFilter::All, 10, 0).unwrap().len(), 3);
        assert_eq!(h.engine.spend_summary().unwrap().spent_microusd, 0);
        // Explicit echoes also preserve the selected mood rather than the source's mood.
        let echo = h.engine.make_echo_or_revisit(elsewhere.id, None).await.unwrap();
        assert_eq!(echo.generation.id, latest.id);
    }
}

#[tokio::test]
async fn service_fallback_skips_disliked_missing_and_corrupt_saved_images() {
    let h = Harness::new();
    h.keywords(&["lighthouse"], &[], &[]);
    let usable = h.generate().await;
    let corrupt = h.generate().await;
    std::fs::write(corrupt.image_path.unwrap(), b"damaged image").unwrap();
    let missing = h.generate().await;
    std::fs::remove_file(missing.image_path.unwrap()).unwrap();
    let disliked = h.generate().await;
    h.engine.mark_shown(disliked.id.clone()).unwrap();
    h.engine.rate(disliked.id, Rating::Disliked).unwrap();
    local_services(&h);
    let shown = h.engine.generate_or_revisit(Trigger::DislikeReplace, None).await.unwrap();
    assert_eq!(shown.generation.id, usable.id);
    assert_eq!(shown.revisit, Some(RevisitReason::ServicesUnavailable));
}

#[tokio::test]
async fn unavailable_services_without_a_saved_image_in_selected_mood_keep_current_wallpaper() {
    let h = Harness::new();
    h.keywords(&["lighthouse"], &[], &[]);
    let current = h.generate().await;
    h.engine.mark_shown(current.id.clone()).unwrap();
    let empty = h.engine.create_mood("Empty".into(), None).unwrap();
    h.engine.set_active_mood(empty.id).unwrap();
    local_services(&h);
    assert!(h.engine.generate_or_revisit(Trigger::Manual, None).await.is_err());
    assert_eq!(h.engine.current().unwrap().unwrap().id, current.id);
    let run = h.engine.run(h.engine.runs(1, 0).unwrap()[0].id.clone()).unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(run.events.iter().filter(|event| event.kind == "service_check").count(), 2);
    assert!(run.events.iter().any(|event| event.kind == "fallback" && event.detail.contains("current wallpaper stays")));
}

#[tokio::test]
async fn repeated_scheduled_availability_failures_revisit_latest_mood_with_backoff_then_recover() {
    let h = Harness::new();
    h.keywords(&["lighthouse"], &[], &[]);
    let latest = h.generate().await;
    h.clock.advance_days(1.0);
    local_services(&h);
    for seconds in [600, 1200] {
        let shown = h.engine.run_if_due(None).await.unwrap().unwrap();
        assert_eq!(shown.generation.id, latest.id);
        assert_eq!(shown.revisit, Some(RevisitReason::ServicesUnavailable));
        let due = h.engine.next_due().unwrap().unwrap();
        assert_eq!(due, h.now() + seconds);
        h.clock.set(due);
    }
    h.update(|settings| { settings.text_provider.kind = ProviderKind::Demo; settings.image_provider.kind = ProviderKind::Demo; });
    let recovered = h.engine.run_if_due(None).await.unwrap().unwrap();
    assert!(recovered.revisit.is_none());
    assert_eq!(h.engine.next_due().unwrap(), Some(h.now() + DAY));
}

#[tokio::test(start_paused = true)]
async fn both_hung_services_are_checked_in_parallel_and_bounded_before_any_paid_work() {
    let dir = tempfile::tempdir().unwrap();
    let http = Arc::new(Hangs::default());
    let engine = open(dir.path(), Arc::new(FixedClock::at(START)), http.clone(), Arc::new(HashingEmbedder), 7);
    let mut settings = engine.settings().unwrap();
    settings.text_provider = ProviderSelection { kind: ProviderKind::Ollama, model: "m".into(), base_url: None };
    settings.image_provider.kind = ProviderKind::ComfyUi;
    engine.update_settings(settings).unwrap();
    let started = tokio::time::Instant::now();
    assert!(matches!(engine.generate(Trigger::Manual, None).await, Err(AutoPaperError::ProviderUnavailable { reason: ProviderUnavailableReason::TimedOut, .. })));
    assert_eq!(started.elapsed(), std::time::Duration::from_secs(8));
    assert_eq!(http.requests.lock().unwrap().len(), 2);
    let run = engine.run(engine.runs(1, 0).unwrap()[0].id.clone()).unwrap();
    assert_eq!(run.events.iter().filter(|event| event.kind == "service_check" && event.detail.contains("8 seconds")).count(), 2);
    assert_eq!(engine.spend_summary().unwrap().spent_microusd, 0);
}


#[tokio::test]
async fn cancellation_drops_both_hung_service_checks_without_a_revisit_or_spend() {
    let dir = tempfile::tempdir().unwrap();
    let http = Arc::new(Hangs::default());
    let engine = open(dir.path(), Arc::new(FixedClock::at(START)), http.clone(), Arc::new(HashingEmbedder), 7);
    engine.add_keyword("lighthouse".into(), KeywordWeight::Must).unwrap();
    let original = engine.generate(Trigger::Manual, None).await.unwrap();
    engine.mark_shown(original.id.clone()).unwrap();
    let mut settings = engine.settings().unwrap();
    settings.text_provider = ProviderSelection { kind: ProviderKind::Ollama, model: "m".into(), base_url: None };
    settings.image_provider.kind = ProviderKind::ComfyUi;
    engine.update_settings(settings).unwrap();
    let task = tokio::spawn({ let engine = engine.clone(); async move { engine.generate_or_revisit(Trigger::Manual, None).await } });
    http.started.notified().await;
    assert_eq!(http.requests.lock().unwrap().len(), 2);
    engine.cancel();
    let outcome = tokio::time::timeout(std::time::Duration::from_millis(500), task).await.unwrap().unwrap();
    assert!(matches!(outcome, Err(AutoPaperError::Cancelled)));
    assert_eq!(engine.current().unwrap().unwrap().id, original.id);
    assert_eq!(engine.runs(1, 0).unwrap()[0].status, RunStatus::Cancelled);
    assert_eq!(engine.spend_summary().unwrap().spent_microusd, 0);
}
