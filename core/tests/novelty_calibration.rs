//! Calibrates novelty against the real embedding model (BAAI/bge-small-en-v1.5). Ignored by default: it
//! needs the model files (`scripts/fetch-model.sh`) and is slow in debug builds. Run it with
//!
//! ```sh
//! scripts/fetch-model.sh
//! cargo test -p autopaper-core --release --test novelty_calibration -- --ignored --nocapture
//! ```
//!
//! `AUTOPAPER_MODEL_DIR` overrides the model directory (default `<repo>/models/bge-small-en-v1.5`).
//!
//! Hand-written concepts, in groups of pairs, as the composer's text model would write them:
//! - **near-duplicates**: the same scene described again in other words — must be judged too similar;
//! - **echoes**: the same place with the weather, time of day, season, age, viewpoint or medium changed —
//!   must fall in the echo band;
//! - **related**: shared keywords, a different scene — must be novel (one known exception, below);
//! - **unrelated**: nothing in common but being wallpapers — must be novel, and below the echo band;
//! - **copies**: an echo request answered with the original and token edits — must be above the echo band.
//!
//! `novelty::DEFAULT_THRESHOLD` sits between the related pairs and the near-duplicates, and
//! `novelty::ECHO_BAND` holds every echo but no copy or unrelated pair. Two overlaps are measured and
//! printed, not asserted away: a related pair whose shared keywords describe the whole scene scores like a
//! near-duplicate (`KNOWN_RELATED_OVERLAPS`), and near-duplicates fall inside the echo band (an echo and a
//! rewording both keep the place). The numbers are recorded on the constants; when the model,
//! `embed::concept_text` or these pairs change, re-run and update both.

use std::path::PathBuf;

use autopaper_core::embed::{CandleEmbedder, HashingEmbedder, concept_text, cosine};
use autopaper_core::model::{Concept, Rating};
use autopaper_core::novelty::{
    self, Calibration, DEFAULT_THRESHOLD, ECHO_BAND, HASHING_ECHO_BAND, HASHING_THRESHOLD, NoveltyPolicy,
};
use autopaper_core::ports::Embedder;
use autopaper_core::store::MemoryRow;

fn model_dir() -> PathBuf {
    std::env::var_os("AUTOPAPER_MODEL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../models/bge-small-en-v1.5")))
}

fn load() -> CandleEmbedder {
    let dir = model_dir();
    CandleEmbedder::load(&dir)
        .unwrap_or_else(|e| panic!("load the model from {} ({e}); run scripts/fetch-model.sh", dir.display()))
}

/// The fields `concept_text` reads; the rest of a concept says how, not what.
struct Scene {
    title: &'static str,
    summary: &'static str,
    setting: &'static str,
    subject: &'static str,
    elements: &'static [&'static str],
    time_of_day: &'static str,
    weather: &'static str,
    season: &'static str,
    mood: &'static [&'static str],
    palette: &'static [&'static str],
}

impl Scene {
    fn concept(&self) -> Concept {
        let list = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        Concept {
            title: self.title.into(),
            summary: self.summary.into(),
            setting: self.setting.into(),
            subject: self.subject.into(),
            elements: list(self.elements),
            time_of_day: self.time_of_day.into(),
            weather: self.weather.into(),
            season: self.season.into(),
            mood: list(self.mood),
            palette: list(self.palette),
            style: "digital painting".into(),
            composition: "wide landscape, calm upper third".into(),
            ..Concept::default()
        }
    }
}

// ── Scenes ────────────────────────────────────────────────────────────────────────────────────────

const OCEAN_TOWERS: Scene = Scene {
    title: "Black ocean, silver structures",
    summary: "A still black ocean at night with tall silver lattice structures rising from the water, their \
              reflections broken by slow swells under a clear starry sky.",
    setting: "open ocean far from any shore",
    subject: "silver lattice structures rising from the sea",
    elements: &["still black water", "silver towers", "starlight reflections", "slow swells"],
    time_of_day: "night",
    weather: "clear",
    season: "",
    mood: &["eerie", "calm", "vast"],
    palette: &["black", "silver", "deep navy"],
};

const OCEAN_TOWERS_REWORDED: Scene = Scene {
    title: "Silver spires on a midnight sea",
    summary: "Gleaming silver spires stand in a dark, motionless sea at midnight, mirrored in the black water \
              beneath a cloudless sky full of stars.",
    setting: "the middle of a dark ocean",
    subject: "metal spires standing in the sea",
    elements: &["mirror-like dark water", "silver spires", "stars reflected in the sea", "gentle swell"],
    time_of_day: "midnight",
    weather: "cloudless",
    season: "",
    mood: &["uncanny", "quiet", "immense"],
    palette: &["jet black", "silver", "navy"],
};

const OCEAN_TOWERS_AFTER_STORM: Scene = Scene {
    title: "Silver structures after the storm",
    summary: "The silver lattice structures rise from a black ocean still heaving after a storm, as the first \
              sunrise light breaks through torn clouds and catches their wet metal.",
    setting: "open ocean far from any shore",
    subject: "silver lattice structures rising from the sea",
    elements: &["rough dark water", "silver towers", "torn storm clouds", "spray", "shafts of sunlight"],
    time_of_day: "sunrise",
    weather: "clearing after a storm",
    season: "",
    mood: &["dramatic", "hopeful", "vast"],
    palette: &["black", "silver", "rose gold", "slate grey"],
};

const TEMPLE_RAIN: Scene = Scene {
    title: "Lanterns in the rainy ruins",
    summary: "Crumbling stone temple ruins in a jungle at night, rain falling through paper lanterns that glow \
              orange along a moss-covered stairway.",
    setting: "overgrown temple ruins in a jungle",
    subject: "a moss-covered stone stairway lit by lanterns",
    elements: &["paper lanterns", "broken stone pillars", "moss", "rain streaks", "puddles"],
    time_of_day: "night",
    weather: "rain",
    season: "",
    mood: &["peaceful", "mysterious"],
    palette: &["deep green", "warm orange", "wet grey stone"],
};

const TEMPLE_RAIN_REWORDED: Scene = Scene {
    title: "Rain on the old temple steps",
    summary: "At night, rain pours over an ancient ruined temple in the jungle, where glowing paper lanterns \
              line mossy stone steps between fallen columns.",
    setting: "an ancient jungle temple in ruins",
    subject: "mossy temple steps lined with glowing lanterns",
    elements: &["glowing lanterns", "fallen columns", "moss", "heavy rain", "wet stone"],
    time_of_day: "night",
    weather: "heavy rain",
    season: "",
    mood: &["tranquil", "enigmatic"],
    palette: &["dark green", "amber", "grey stone"],
};

const TEMPLE_SNOW_NOON: Scene = Scene {
    title: "The temple ruins under snow",
    summary: "The same moss-covered temple stairway in the jungle ruins, now hushed under fresh snow at noon, \
              the paper lanterns dark and capped with white.",
    setting: "overgrown temple ruins in a jungle",
    subject: "a snow-covered stone stairway with unlit lanterns",
    elements: &["unlit paper lanterns", "broken stone pillars", "fresh snow", "frosted moss"],
    time_of_day: "noon",
    weather: "snow",
    season: "winter",
    mood: &["hushed", "serene"],
    palette: &["white", "muted green", "pale grey"],
};

const LAVENDER_SUNSET: Scene = Scene {
    title: "Lavender rows at sunset",
    summary: "Endless rows of blooming lavender lead to an old stone farmhouse on a hill, glowing in the warm \
              light of a summer sunset.",
    setting: "rolling lavender fields in Provence",
    subject: "an old stone farmhouse on a hill",
    elements: &["lavender rows", "stone farmhouse", "cypress trees", "long shadows"],
    time_of_day: "sunset",
    weather: "clear",
    season: "summer",
    mood: &["warm", "nostalgic", "peaceful"],
    palette: &["purple", "gold", "soft pink"],
};

const LAVENDER_SUNSET_REWORDED: Scene = Scene {
    title: "Farmhouse among the lavender",
    summary: "On a summer evening the setting sun bathes a hilltop stone farmhouse and the long purple lines of \
              flowering lavender around it.",
    setting: "Provençal lavender fields",
    subject: "a stone farmhouse on a hilltop",
    elements: &["purple lavender lines", "farmhouse", "tall cypresses", "evening shadows"],
    time_of_day: "evening",
    weather: "clear sky",
    season: "summer",
    mood: &["golden", "wistful", "calm"],
    palette: &["violet", "amber", "pink"],
};

const LAVENDER_WINTER_DAWN: Scene = Scene {
    title: "The lavender farm in frost",
    summary: "The stone farmhouse on its hill at a frosty winter dawn, the lavender rows cut back to silver-grey \
              mounds that run toward it under a pale sky.",
    setting: "rolling lavender fields in Provence",
    subject: "an old stone farmhouse on a hill",
    elements: &["pruned lavender rows", "stone farmhouse", "cypress trees", "hoarfrost"],
    time_of_day: "dawn",
    weather: "frost",
    season: "winter",
    mood: &["quiet", "crisp", "patient"],
    palette: &["silver grey", "pale blue", "muted purple"],
};

const CABIN_SNOW_DAWN: Scene = Scene {
    title: "Cabin below the peaks",
    summary: "A small log cabin with a smoking chimney sits in deep snow beneath jagged mountain peaks lit pink \
              by the winter dawn.",
    setting: "a snowy alpine valley",
    subject: "a log cabin with a smoking chimney",
    elements: &["log cabin", "chimney smoke", "snow-laden pines", "jagged peaks"],
    time_of_day: "dawn",
    weather: "clear and cold",
    season: "winter",
    mood: &["cosy", "still", "remote"],
    palette: &["white", "pink", "pine green", "warm brown"],
};

const CABIN_SNOW_DAWN_REWORDED: Scene = Scene {
    title: "First light on the mountain hut",
    summary: "At first light in winter, smoke curls from a lone wooden hut buried in snow, the sharp summits \
              above it blushing pink.",
    setting: "a mountain valley deep in snow",
    subject: "a wooden hut with smoke rising from its chimney",
    elements: &["wooden hut", "smoke", "snowy fir trees", "sharp summits"],
    time_of_day: "early morning",
    weather: "cold and clear",
    season: "winter",
    mood: &["snug", "quiet", "isolated"],
    palette: &["snow white", "rose", "dark green", "brown"],
};

const CABIN_SUMMER: Scene = Scene {
    title: "The cabin in high summer",
    summary: "The log cabin beneath the jagged peaks on a summer afternoon, the valley green with wildflowers \
              and the last snow only on the summits.",
    setting: "an alpine valley",
    subject: "a log cabin below jagged peaks",
    elements: &["log cabin", "wildflower meadow", "pines", "jagged peaks", "a stream"],
    time_of_day: "afternoon",
    weather: "sunny",
    season: "summer",
    mood: &["bright", "open", "remote"],
    palette: &["green", "sky blue", "yellow", "warm brown"],
};

const NEON_ALLEY: Scene = Scene {
    title: "Neon alley in the rain",
    summary: "A narrow city alley at night shines with neon signs reflected in rain puddles, steam drifting \
              from vents between crowded shopfronts.",
    setting: "a narrow alley in a dense city",
    subject: "rain-soaked alley lit by neon",
    elements: &["neon signs", "puddle reflections", "steam vents", "shop awnings", "power lines"],
    time_of_day: "night",
    weather: "rain",
    season: "",
    mood: &["moody", "electric"],
    palette: &["magenta", "cyan", "deep blue", "black"],
};

const NEON_ALLEY_REWORDED: Scene = Scene {
    title: "Rainy backstreet glow",
    summary: "Neon light from crowded shop signs spills across the wet pavement of a tight city backstreet at \
              night, with steam rising and rain still falling.",
    setting: "a cramped city backstreet",
    subject: "a wet backstreet glowing with neon signs",
    elements: &["glowing signs", "wet pavement reflections", "rising steam", "awnings", "overhead cables"],
    time_of_day: "night",
    weather: "rainy",
    season: "",
    mood: &["atmospheric", "vibrant"],
    palette: &["pink", "teal", "dark blue", "black"],
};

const NEON_ALLEY_ABANDONED: Scene = Scene {
    title: "The neon alley, decades on",
    summary: "The same narrow city alley years later on a bright afternoon: the neon signs are dark and \
              cracked, vines climb the shopfronts and grass grows between the paving stones.",
    setting: "a narrow alley in a dense city",
    subject: "an abandoned alley reclaimed by plants",
    elements: &["dead neon signs", "climbing vines", "grass in the cracks", "rusted awnings", "power lines"],
    time_of_day: "afternoon",
    weather: "sunny",
    season: "",
    mood: &["melancholy", "quiet"],
    palette: &["green", "rust", "faded pink", "sunlit grey"],
};

const DUNES_STARS: Scene = Scene {
    title: "Milky Way over the dunes",
    summary: "Smooth sand dunes curve under the Milky Way on a moonless night, a lone acacia tree silhouetted \
              against the stars.",
    setting: "a vast sand desert",
    subject: "the Milky Way above sand dunes",
    elements: &["sand dunes", "Milky Way", "lone acacia tree", "countless stars"],
    time_of_day: "night",
    weather: "clear",
    season: "",
    mood: &["awe", "solitude"],
    palette: &["indigo", "violet", "sand beige"],
};

const DUNES_STARS_REWORDED: Scene = Scene {
    title: "Starlit desert",
    summary: "Under a moonless sky the band of the galaxy arcs over rippled desert dunes, where a single \
              thorn tree stands in silhouette.",
    setting: "an endless desert of dunes",
    subject: "the galaxy arching over the dunes",
    elements: &["rippled dunes", "band of the galaxy", "single thorn tree", "starfield"],
    time_of_day: "night",
    weather: "clear sky",
    season: "",
    mood: &["wonder", "loneliness"],
    palette: &["deep indigo", "purple", "beige"],
};

const DUNES_SANDSTORM: Scene = Scene {
    title: "Sandstorm over the dunes",
    summary: "The same curving dunes at midday as a sandstorm rolls in, the lone acacia bending in the wind \
              and the sun a pale disc behind the dust.",
    setting: "a vast sand desert",
    subject: "a sandstorm sweeping over the dunes",
    elements: &["sand dunes", "wall of dust", "lone acacia tree", "pale sun"],
    time_of_day: "midday",
    weather: "sandstorm",
    season: "",
    mood: &["harsh", "dramatic", "solitude"],
    palette: &["ochre", "dusty orange", "pale yellow"],
};

const MISTY_LAKE: Scene = Scene {
    title: "Canoe on a misty lake",
    summary: "A red canoe rests at the edge of a glassy lake wrapped in morning mist, dark pines standing on \
              the far shore.",
    setting: "a mountain lake in a pine forest",
    subject: "a red canoe on still water",
    elements: &["red canoe", "morning mist", "pine trees", "mirror-still water", "wooden dock"],
    time_of_day: "morning",
    weather: "mist",
    season: "",
    mood: &["serene", "quiet"],
    palette: &["soft grey", "dark green", "red"],
};

const MISTY_LAKE_REWORDED: Scene = Scene {
    title: "Morning fog on the pine lake",
    summary: "Fog drifts over a perfectly still lake in the early morning, where a red canoe is tied to a small \
              wooden jetty below the pines.",
    setting: "a pine-ringed lake",
    subject: "a red canoe tied to a jetty",
    elements: &["red canoe", "fog", "pines", "still water", "wooden jetty"],
    time_of_day: "early morning",
    weather: "foggy",
    season: "",
    mood: &["calm", "hushed"],
    palette: &["pale grey", "forest green", "red"],
};

const LAKE_AUTUMN_EVENING: Scene = Scene {
    title: "The pine lake in autumn",
    summary: "The red canoe at its wooden dock on an autumn evening, the far shore ablaze with golden birches \
              among the pines and the lake catching the low sun.",
    setting: "a mountain lake in a pine forest",
    subject: "a red canoe at a wooden dock",
    elements: &["red canoe", "golden birches", "pine trees", "wooden dock", "rippled water"],
    time_of_day: "evening",
    weather: "clear",
    season: "autumn",
    mood: &["warm", "reflective"],
    palette: &["gold", "orange", "dark green", "red"],
};

const CORAL_REEF: Scene = Scene {
    title: "Sunbeams over the reef",
    summary: "Shafts of sunlight fall through clear turquoise water onto a vivid coral reef where small \
              tropical fish dart between branching corals.",
    setting: "a shallow tropical coral reef",
    subject: "branching corals in sunbeams",
    elements: &["branching corals", "tropical fish", "sun rays", "sandy patches"],
    time_of_day: "midday",
    weather: "",
    season: "",
    mood: &["lively", "bright"],
    palette: &["turquoise", "coral pink", "yellow"],
};

const CORAL_REEF_REWORDED: Scene = Scene {
    title: "Light on the coral garden",
    summary: "Beams of daylight slant down through bright blue-green water over a colourful reef, schools of \
              little fish weaving among the coral branches.",
    setting: "a sunlit coral reef in warm shallows",
    subject: "a colourful coral garden lit from above",
    elements: &["coral branches", "schools of small fish", "light beams", "white sand"],
    time_of_day: "day",
    weather: "",
    season: "",
    mood: &["vibrant", "cheerful"],
    palette: &["aqua", "pink", "golden yellow"],
};

const MAPLE_GARDEN: Scene = Scene {
    title: "Maple bridge in autumn",
    summary: "A red wooden bridge arches over a koi pond in a Japanese garden, framed by maples in blazing \
              autumn red.",
    setting: "a Japanese garden",
    subject: "a red arched bridge over a pond",
    elements: &["arched bridge", "red maples", "koi pond", "stone lantern", "fallen leaves"],
    time_of_day: "afternoon",
    weather: "clear",
    season: "autumn",
    mood: &["tranquil", "graceful"],
    palette: &["crimson", "orange", "moss green"],
};

const MAPLE_GARDEN_REWORDED: Scene = Scene {
    title: "Red leaves over the koi pond",
    summary: "In a Japanese garden in autumn, flaming red maple leaves surround a curved vermilion bridge \
              crossing a pond full of koi.",
    setting: "a traditional Japanese garden",
    subject: "a curved vermilion bridge over a koi pond",
    elements: &["vermilion bridge", "maple trees", "koi", "stone lantern", "leaves on the water"],
    time_of_day: "afternoon",
    weather: "sunny",
    season: "autumn",
    mood: &["peaceful", "elegant"],
    palette: &["red", "amber", "green"],
};

const FLOATING_ISLANDS: Scene = Scene {
    title: "Islands in the sky",
    summary: "Grassy floating islands drift among white clouds, waterfalls spilling from their edges into the \
              open sky on a bright day.",
    setting: "a sky full of floating islands",
    subject: "floating islands with waterfalls",
    elements: &["floating islands", "waterfalls", "clouds", "small trees", "birds"],
    time_of_day: "day",
    weather: "fair",
    season: "spring",
    mood: &["dreamlike", "uplifting"],
    palette: &["sky blue", "bright green", "white"],
};

const FLOATING_ISLANDS_REWORDED: Scene = Scene {
    title: "Waterfalls from floating land",
    summary: "On a sunny day, green islands hover in the sky between soft clouds, with streams pouring off \
              their edges as long waterfalls into the air.",
    setting: "an open sky with hovering islands",
    subject: "hovering green islands pouring waterfalls",
    elements: &["hovering islands", "long waterfalls", "soft clouds", "trees", "flying birds"],
    time_of_day: "daytime",
    weather: "sunny",
    season: "spring",
    mood: &["fantastical", "joyful"],
    palette: &["blue", "green", "cloud white"],
};

const FLOATING_ISLANDS_FROZEN: Scene = Scene {
    title: "The sky islands in winter",
    summary: "The floating islands at night in deep winter, their waterfalls frozen into hanging pillars of \
              ice under a green aurora.",
    setting: "a sky full of floating islands",
    subject: "floating islands with frozen waterfalls",
    elements: &["floating islands", "frozen waterfalls", "aurora", "snow-covered trees", "stars"],
    time_of_day: "night",
    weather: "aurora, clear and cold",
    season: "winter",
    mood: &["dreamlike", "still", "cold"],
    palette: &["teal", "ice blue", "white", "black"],
};

const LIGHTHOUSE_STORM: Scene = Scene {
    title: "Lighthouse in the storm",
    summary: "A lone lighthouse on a sea cliff sends its beam through a storm at night as huge waves break \
              white against the rocks below.",
    setting: "a rocky sea cliff",
    subject: "a lighthouse with its beam sweeping",
    elements: &["lighthouse", "crashing waves", "rain", "lightning", "cliff"],
    time_of_day: "night",
    weather: "storm",
    season: "autumn",
    mood: &["dramatic", "defiant"],
    palette: &["slate", "white", "storm blue", "yellow beam"],
};

const LIGHTHOUSE_CALM: Scene = Scene {
    title: "The lighthouse, restored",
    summary: "The lighthouse on its sea cliff years later on a calm summer morning, freshly painted, with \
              wildflowers on the cliff top and a glassy sea below.",
    setting: "a rocky sea cliff",
    subject: "a freshly painted lighthouse",
    elements: &["lighthouse", "calm sea", "wildflowers", "cliff", "gulls"],
    time_of_day: "morning",
    weather: "calm and sunny",
    season: "summer",
    mood: &["peaceful", "renewed"],
    palette: &["white", "red", "sea blue", "green"],
};

const BRIDGE_SPRING: Scene = Scene {
    title: "Stone bridge in spring",
    summary: "An old arched stone bridge crosses a clear river in a green valley, cherry trees in blossom along \
              the banks on a spring morning.",
    setting: "a river valley in the countryside",
    subject: "an old arched stone bridge",
    elements: &["stone bridge", "clear river", "cherry blossoms", "meadow"],
    time_of_day: "morning",
    weather: "sunny",
    season: "spring",
    mood: &["fresh", "gentle"],
    palette: &["pink", "fresh green", "stone grey"],
};

const BRIDGE_RUINED: Scene = Scene {
    title: "The stone bridge, fallen",
    summary: "The old arched stone bridge in its river valley generations later, its middle arch collapsed into \
              the water and ivy over the piers, in autumn fog at dusk.",
    setting: "a river valley in the countryside",
    subject: "a collapsed arched stone bridge",
    elements: &["broken stone bridge", "river", "ivy", "bare trees", "fallen stones"],
    time_of_day: "dusk",
    weather: "fog",
    season: "autumn",
    mood: &["melancholy", "timeless"],
    palette: &["grey", "rust", "dark green"],
};

const ROMAN_FORUM_RAIN: Scene = Scene {
    title: "Rain on the forum",
    summary: "Grey rain falls over the marble columns and broken arches of an ancient Roman forum at midday, \
              umbrellas of a few visitors far off between the stones.",
    setting: "the ruins of a Roman forum in a city",
    subject: "marble columns and triumphal arches",
    elements: &["marble columns", "broken arches", "cobblestones", "umbrellas", "pigeons"],
    time_of_day: "midday",
    weather: "rain",
    season: "",
    mood: &["contemplative", "grand"],
    palette: &["marble white", "terracotta", "grey"],
};

const LANTERN_FESTIVAL: Scene = Scene {
    title: "Lanterns on the river",
    summary: "Hundreds of floating lanterns drift down a wide river during a festival night, their light \
              doubled in the water beneath a stone embankment crowded with people.",
    setting: "a river through an old town",
    subject: "floating lanterns on a river",
    elements: &["floating lanterns", "crowd", "embankment", "reflections", "boats"],
    time_of_day: "night",
    weather: "clear",
    season: "",
    mood: &["festive", "warm"],
    palette: &["gold", "deep blue", "red"],
};

const TROPICAL_LAGOON: Scene = Scene {
    title: "Pier over the lagoon",
    summary: "A wooden pier runs out over a turquoise ocean lagoon on a sunny afternoon, palm trees leaning \
              over white sand.",
    setting: "a tropical island lagoon",
    subject: "a wooden pier over turquoise water",
    elements: &["wooden pier", "palm trees", "white sand", "turquoise ocean"],
    time_of_day: "afternoon",
    weather: "sunny",
    season: "summer",
    mood: &["relaxed", "bright"],
    palette: &["turquoise", "white", "palm green"],
};

const SNOWY_TRAM_STREET: Scene = Scene {
    title: "Tram through the snow",
    summary: "An old yellow tram rolls down a snowy city street at dusk, shop windows glowing warm and snow \
              falling through the street lamps.",
    setting: "a European city street",
    subject: "a yellow tram in falling snow",
    elements: &["yellow tram", "street lamps", "shop windows", "falling snow", "tram tracks"],
    time_of_day: "dusk",
    weather: "snowfall",
    season: "winter",
    mood: &["cosy", "bustling"],
    palette: &["yellow", "blue grey", "warm orange", "white"],
};

const REDWOODS: Scene = Scene {
    title: "Among the redwoods",
    summary: "Giant redwood trunks rise from a forest floor of ferns as beams of sunlight cut through the \
              morning haze high in the canopy.",
    setting: "an old-growth redwood forest",
    subject: "giant redwood trunks",
    elements: &["redwood trunks", "ferns", "sunbeams", "forest floor", "haze"],
    time_of_day: "morning",
    weather: "hazy",
    season: "",
    mood: &["majestic", "humbling"],
    palette: &["red brown", "fern green", "gold"],
};

const HARBOUR_NIGHT: Scene = Scene {
    title: "Harbour lights",
    summary: "Fishing boats rock gently in a small harbour town at night, string lights along the quay and \
              lit windows stacked up the hillside.",
    setting: "a seaside harbour town",
    subject: "fishing boats in a lit harbour",
    elements: &["fishing boats", "string lights", "quay", "hillside houses", "reflections"],
    time_of_day: "night",
    weather: "calm",
    season: "summer",
    mood: &["cosy", "quiet"],
    palette: &["navy", "warm yellow", "white"],
};

const SLOT_CANYON: Scene = Scene {
    title: "Light in the slot canyon",
    summary: "A beam of midday sun falls into a narrow sandstone slot canyon, lighting its wave-shaped red \
              walls and drifting dust.",
    setting: "a desert slot canyon",
    subject: "a light beam in a narrow canyon",
    elements: &["sandstone walls", "light beam", "drifting dust", "sand floor"],
    time_of_day: "midday",
    weather: "clear",
    season: "",
    mood: &["sacred", "intimate"],
    palette: &["red orange", "purple shadow", "gold"],
};

const ALPINE_GLACIER: Scene = Scene {
    title: "Glacier lake below the peaks",
    summary: "A milky turquoise glacier lake lies beneath snow-capped mountain peaks and a hanging glacier on a \
              crisp clear day.",
    setting: "a high alpine basin",
    subject: "a glacier lake beneath snowy peaks",
    elements: &["glacier lake", "snow-capped peaks", "hanging glacier", "scree slopes"],
    time_of_day: "day",
    weather: "clear",
    season: "summer",
    mood: &["pristine", "majestic"],
    palette: &["turquoise", "white", "granite grey"],
};

const RICE_TERRACES: Scene = Scene {
    title: "Terraces in the mist",
    summary: "Green rice terraces step down steep mountain slopes into a valley of drifting mist, a few \
              farmhouses tucked among them at dawn.",
    setting: "mountain rice terraces",
    subject: "stepped green terraces in mist",
    elements: &["rice terraces", "mist", "farmhouses", "mountain slopes"],
    time_of_day: "dawn",
    weather: "misty",
    season: "summer",
    mood: &["calm", "timeless"],
    palette: &["emerald", "soft white", "earth brown"],
};

const JUNGLE_WATERFALL: Scene = Scene {
    title: "Hidden jungle waterfall",
    summary: "A tall waterfall plunges into a clear green pool hidden in a tropical jungle, vines and ferns \
              hanging over wet black rock.",
    setting: "a tropical rainforest",
    subject: "a waterfall falling into a jungle pool",
    elements: &["waterfall", "green pool", "vines", "ferns", "black rock"],
    time_of_day: "afternoon",
    weather: "humid",
    season: "",
    mood: &["lush", "secluded"],
    palette: &["emerald", "jade", "black"],
};

const NEW_ENGLAND_AUTUMN: Scene = Scene {
    title: "Village in the fall",
    summary: "A white wooden church steeple rises over a small New England village surrounded by hills of \
              orange and yellow autumn trees.",
    setting: "a New England village",
    subject: "a white church steeple among autumn trees",
    elements: &["white church", "autumn trees", "village green", "rolling hills"],
    time_of_day: "afternoon",
    weather: "crisp and clear",
    season: "autumn",
    mood: &["cosy", "nostalgic"],
    palette: &["orange", "yellow", "white", "red"],
};

const SATURN_FROM_MOON: Scene = Scene {
    title: "Saturn over an icy moon",
    summary: "Saturn and its rings hang huge in a black sky above the cracked ice plains of one of its moons, \
              with distant stars.",
    setting: "the icy surface of a moon of Saturn",
    subject: "Saturn and its rings",
    elements: &["Saturn", "rings", "ice plains", "cracks", "stars"],
    time_of_day: "",
    weather: "",
    season: "",
    mood: &["cosmic", "silent"],
    palette: &["black", "pale gold", "ice blue"],
};

const TULIP_WINDMILL: Scene = Scene {
    title: "Tulips and the windmill",
    summary: "Stripes of red and yellow tulips run toward a Dutch windmill under a bright spring sky with \
              puffy clouds.",
    setting: "Dutch tulip fields",
    subject: "a windmill beyond tulip rows",
    elements: &["tulip rows", "windmill", "canal", "puffy clouds"],
    time_of_day: "morning",
    weather: "sunny",
    season: "spring",
    mood: &["cheerful", "fresh"],
    palette: &["red", "yellow", "sky blue"],
};

const PARIS_CAFE: Scene = Scene {
    title: "Café corner in Paris",
    summary: "A Parisian café corner with wicker chairs and a striped awning on a spring afternoon, chestnut \
              trees in leaf along the boulevard.",
    setting: "a boulevard in Paris",
    subject: "a café terrace on a corner",
    elements: &["wicker chairs", "striped awning", "chestnut trees", "cobblestones"],
    time_of_day: "afternoon",
    weather: "mild",
    season: "spring",
    mood: &["charming", "leisurely"],
    palette: &["cream", "green", "red"],
};

const GREENHOUSE: Scene = Scene {
    title: "Inside the glasshouse",
    summary: "A Victorian glass greenhouse crowded with tropical plants and palms, warm light filtering through \
              the iron-framed panes.",
    setting: "a Victorian greenhouse",
    subject: "tropical plants under a glass dome",
    elements: &["iron frames", "glass panes", "palms", "hanging ferns", "tiled path"],
    time_of_day: "afternoon",
    weather: "",
    season: "",
    mood: &["lush", "warm"],
    palette: &["green", "white", "warm gold"],
};

const ICE_FLOES: Scene = Scene {
    title: "Polar bear on the floes",
    summary: "A polar bear crosses broken sea ice floes under a low arctic sun, the dark water between them \
              still and cold.",
    setting: "the Arctic Ocean",
    subject: "a polar bear on sea ice",
    elements: &["polar bear", "ice floes", "dark water", "low sun"],
    time_of_day: "low sun",
    weather: "clear and freezing",
    season: "",
    mood: &["wild", "lonely"],
    palette: &["white", "ice blue", "pale orange"],
};

const KELP_FOREST: Scene = Scene {
    title: "Kelp forest",
    summary: "Tall golden kelp sways in cool green water, a sea lion gliding between the stalks as light ripples \
              down from the surface.",
    setting: "an underwater kelp forest off a rocky coast",
    subject: "towering kelp stalks",
    elements: &["kelp", "sea lion", "rippling light", "rocks"],
    time_of_day: "day",
    weather: "",
    season: "",
    mood: &["calm", "otherworldly"],
    palette: &["olive green", "gold", "teal"],
};

const SUNFLOWERS: Scene = Scene {
    title: "Sunflower field",
    summary: "A field of tall sunflowers faces the late summer sun, a red barn in the distance under a wide blue \
              sky.",
    setting: "farmland",
    subject: "a field of sunflowers",
    elements: &["sunflowers", "red barn", "wide sky", "fence"],
    time_of_day: "afternoon",
    weather: "sunny",
    season: "late summer",
    mood: &["happy", "warm"],
    palette: &["yellow", "blue", "red"],
};

// Echoes along the viewpoint and medium axes. Style is not part of `concept_text`, so a change of medium
// shows only through the title, summary and whatever else the model changes with it.

const MAPLE_GARDEN_WINTER_ABOVE: Scene = Scene {
    title: "The maple garden from above, in snow",
    summary: "The Japanese garden seen from high above in winter: the red bridge a thin curve across the \
              frozen koi pond, the bare maples dusted with snow.",
    setting: "a Japanese garden",
    subject: "a red arched bridge over a frozen pond, seen from above",
    elements: &["arched bridge", "bare maples", "frozen pond", "stone lantern", "snow-covered paths"],
    time_of_day: "morning",
    weather: "light snow",
    season: "winter",
    mood: &["quiet", "graphic"],
    palette: &["white", "crimson", "charcoal"],
};

const CORAL_REEF_PRINT_NIGHT: Scene = Scene {
    title: "The reef by moonlight, as a print",
    summary: "The coral reef at night rendered as a woodblock print: branching corals in flat bands of colour \
              under a moonlit surface, a few fish asleep among them.",
    setting: "a shallow tropical coral reef",
    subject: "branching corals under moonlight",
    elements: &["branching corals", "sleeping fish", "moonlit surface", "sandy patches"],
    time_of_day: "night",
    weather: "",
    season: "",
    mood: &["calm", "stylised"],
    palette: &["indigo", "coral pink", "pale gold"],
};

// More scenes sharing keywords (as Musts make every wallpaper do): the everyday test of the threshold.

const ABBEY_RAIN: Scene = Scene {
    title: "Rain over the abbey ruins",
    summary: "Rain sweeps across the roofless ruins of a Gothic abbey at dusk, its pointed arches standing \
              open to a heavy grey sky as crows circle overhead.",
    setting: "the ruins of a Gothic abbey in open countryside",
    subject: "roofless pointed arches",
    elements: &["pointed arches", "empty rose window", "wet grass", "crows", "rain curtains"],
    time_of_day: "dusk",
    weather: "rain",
    season: "autumn",
    mood: &["brooding", "solemn"],
    palette: &["slate grey", "moss green", "pale gold"],
};

const RAINY_OVERPASS: Scene = Scene {
    title: "Neon overpass in the rain",
    summary: "Rain falls on a tangle of city overpasses at night, car lights streaking red and white beneath \
              towers covered in neon signs.",
    setting: "a megacity highway interchange",
    subject: "rain-slick overpasses and light trails",
    elements: &["overpasses", "light trails", "neon signs", "skyscrapers", "rain"],
    time_of_day: "night",
    weather: "rain",
    season: "",
    mood: &["restless", "electric"],
    palette: &["magenta", "red", "cyan", "black"],
};

const GLOWING_BEACH: Scene = Scene {
    title: "Glowing surf at night",
    summary: "Waves break in bright blue bioluminescence along a dark ocean beach at night, the stars \
              overhead and wet sand mirroring the glow.",
    setting: "an ocean beach",
    subject: "bioluminescent waves",
    elements: &["glowing waves", "wet sand", "stars", "dark headland"],
    time_of_day: "night",
    weather: "clear",
    season: "summer",
    mood: &["magical", "calm"],
    palette: &["electric blue", "black", "silver"],
};

// Copies: the original handed back with token changes, as a text model might when asked for an echo
// and failing to transform it. The echo band's upper bound is there to reject these.

const OCEAN_TOWERS_COPY: Scene = Scene {
    title: "Silver structures on the black ocean",
    summary: "A still black ocean at night with tall silver lattice structures rising from the water, their \
              reflections broken by slow swells under a starry sky with a few thin clouds.",
    weather: "thin clouds",
    ..OCEAN_TOWERS
};

const TEMPLE_RAIN_COPY: Scene = Scene {
    title: "Lanterns in the rain-soaked ruins",
    summary: "Crumbling stone temple ruins in a jungle at night, a light drizzle falling through paper lanterns \
              that glow orange along a moss-covered stairway.",
    weather: "drizzle",
    ..TEMPLE_RAIN
};

const LAVENDER_SUNSET_COPY: Scene = Scene {
    title: "Lavender rows in the golden hour",
    summary: "Endless rows of blooming lavender lead to an old stone farmhouse on a hill, glowing in the warm \
              golden-hour light of a summer evening.",
    time_of_day: "golden hour",
    ..LAVENDER_SUNSET
};

const CABIN_SNOW_DAWN_COPY: Scene = Scene {
    title: "Cabin below the peaks at sunrise",
    summary: "A small log cabin with a smoking chimney sits in deep snow beneath jagged mountain peaks lit pink \
              by the winter sunrise.",
    time_of_day: "sunrise",
    ..CABIN_SNOW_DAWN
};

const NEON_ALLEY_COPY: Scene = Scene {
    title: "Neon alley in light rain",
    summary: "A narrow city alley late at night shines with neon signs reflected in puddles from a light rain, \
              steam drifting from vents between crowded shopfronts.",
    time_of_day: "late night",
    weather: "light rain",
    ..NEON_ALLEY
};

const DUNES_STARS_COPY: Scene = Scene {
    title: "Milky Way above the dunes",
    summary: "Smooth sand dunes curve under the Milky Way beside a thin crescent moon, a lone acacia tree \
              silhouetted against the stars.",
    weather: "clear, thin crescent moon",
    ..DUNES_STARS
};

const MISTY_LAKE_COPY: Scene = Scene {
    title: "Canoe on a misty lake at dawn",
    summary: "A red canoe rests at the edge of a glassy lake wrapped in light dawn mist, dark pines standing \
              on the far shore.",
    time_of_day: "dawn",
    weather: "light mist",
    ..MISTY_LAKE
};

const MAPLE_GARDEN_COPY: Scene = Scene {
    title: "Maple bridge on an autumn afternoon",
    summary: "A red wooden bridge arches over a koi pond in a Japanese garden late in the afternoon, framed by \
              maples in blazing autumn red.",
    time_of_day: "late afternoon",
    ..MAPLE_GARDEN
};

// ── Pairs ─────────────────────────────────────────────────────────────────────────────────────────

struct Pair {
    label: &'static str,
    a: &'static Scene,
    b: &'static Scene,
}

const fn pair(label: &'static str, a: &'static Scene, b: &'static Scene) -> Pair {
    Pair { label, a, b }
}

const NEAR_DUPLICATES: &[Pair] = &[
    pair("ocean towers / reworded", &OCEAN_TOWERS, &OCEAN_TOWERS_REWORDED),
    pair("temple in rain / reworded", &TEMPLE_RAIN, &TEMPLE_RAIN_REWORDED),
    pair("lavender sunset / reworded", &LAVENDER_SUNSET, &LAVENDER_SUNSET_REWORDED),
    pair("cabin at dawn / reworded", &CABIN_SNOW_DAWN, &CABIN_SNOW_DAWN_REWORDED),
    pair("neon alley / reworded", &NEON_ALLEY, &NEON_ALLEY_REWORDED),
    pair("dunes and stars / reworded", &DUNES_STARS, &DUNES_STARS_REWORDED),
    pair("misty lake / reworded", &MISTY_LAKE, &MISTY_LAKE_REWORDED),
    pair("coral reef / reworded", &CORAL_REEF, &CORAL_REEF_REWORDED),
    pair("maple garden / reworded", &MAPLE_GARDEN, &MAPLE_GARDEN_REWORDED),
    pair("floating islands / reworded", &FLOATING_ISLANDS, &FLOATING_ISLANDS_REWORDED),
];

const ECHOES: &[Pair] = &[
    pair("ocean towers: night → after a storm at sunrise", &OCEAN_TOWERS, &OCEAN_TOWERS_AFTER_STORM),
    pair("temple: rainy night → snowy noon in winter", &TEMPLE_RAIN, &TEMPLE_SNOW_NOON),
    pair("lavender farm: summer sunset → winter frost at dawn", &LAVENDER_SUNSET, &LAVENDER_WINTER_DAWN),
    pair("cabin: winter dawn → summer afternoon", &CABIN_SNOW_DAWN, &CABIN_SUMMER),
    pair("neon alley: rainy night → abandoned, overgrown", &NEON_ALLEY, &NEON_ALLEY_ABANDONED),
    pair("dunes: starry night → midday sandstorm", &DUNES_STARS, &DUNES_SANDSTORM),
    pair("pine lake: misty morning → autumn evening", &MISTY_LAKE, &LAKE_AUTUMN_EVENING),
    pair("sky islands: spring day → frozen night, aurora", &FLOATING_ISLANDS, &FLOATING_ISLANDS_FROZEN),
    pair("lighthouse: storm at night → restored, calm summer", &LIGHTHOUSE_STORM, &LIGHTHOUSE_CALM),
    pair("stone bridge: spring → collapsed, autumn fog", &BRIDGE_SPRING, &BRIDGE_RUINED),
    pair("maple garden: autumn → from above, in snow", &MAPLE_GARDEN, &MAPLE_GARDEN_WINTER_ABOVE),
    pair("coral reef: midday → moonlit woodblock print", &CORAL_REEF, &CORAL_REEF_PRINT_NIGHT),
];

/// Related pairs that score like near-duplicates: their shared keywords, mood and palette describe the
/// whole scene, so only setting and subject differ. Measured, printed, and documented on
/// `novelty::DEFAULT_THRESHOLD`; the test fails if one stops overlapping, so this list stays true.
const KNOWN_RELATED_OVERLAPS: &[&str] = &["neon + rain + night: alley / overpass"];

const COPIES: &[Pair] = &[
    pair("ocean towers: + thin clouds", &OCEAN_TOWERS, &OCEAN_TOWERS_COPY),
    pair("temple: rain → drizzle", &TEMPLE_RAIN, &TEMPLE_RAIN_COPY),
    pair("lavender: sunset → golden hour", &LAVENDER_SUNSET, &LAVENDER_SUNSET_COPY),
    pair("cabin: dawn → sunrise", &CABIN_SNOW_DAWN, &CABIN_SNOW_DAWN_COPY),
    pair("neon alley: rain → light rain, late night", &NEON_ALLEY, &NEON_ALLEY_COPY),
    pair("dunes: + crescent moon", &DUNES_STARS, &DUNES_STARS_COPY),
    pair("misty lake: morning → dawn", &MISTY_LAKE, &MISTY_LAKE_COPY),
    pair("maple garden: afternoon → late afternoon", &MAPLE_GARDEN, &MAPLE_GARDEN_COPY),
];

const RELATED: &[Pair] = &[
    pair("rain + ruins: jungle temple / Roman forum", &TEMPLE_RAIN, &ROMAN_FORUM_RAIN),
    pair("lanterns: rainy temple / river festival", &TEMPLE_RAIN, &LANTERN_FESTIVAL),
    pair("ocean: silver towers / tropical lagoon", &OCEAN_TOWERS, &TROPICAL_LAGOON),
    pair("snow: mountain cabin / city tram", &CABIN_SNOW_DAWN, &SNOWY_TRAM_STREET),
    pair("forest: misty pine lake / redwoods", &MISTY_LAKE, &REDWOODS),
    pair("night lights: neon alley / harbour town", &NEON_ALLEY, &HARBOUR_NIGHT),
    pair("desert: starry dunes / slot canyon", &DUNES_STARS, &SLOT_CANYON),
    pair("mountains: glacier lake / rice terraces", &ALPINE_GLACIER, &RICE_TERRACES),
    pair("waterfall: sky islands / jungle", &FLOATING_ISLANDS, &JUNGLE_WATERFALL),
    pair("autumn: maple garden / New England village", &MAPLE_GARDEN, &NEW_ENGLAND_AUTUMN),
    pair("rain + ruins: Roman forum / Gothic abbey", &ROMAN_FORUM_RAIN, &ABBEY_RAIN),
    pair("rain + ruins: jungle temple / Gothic abbey", &TEMPLE_RAIN, &ABBEY_RAIN),
    pair("neon + rain + night: alley / overpass", &NEON_ALLEY, &RAINY_OVERPASS),
    pair("ocean + night: silver towers / glowing surf", &OCEAN_TOWERS, &GLOWING_BEACH),
];

const UNRELATED: &[Pair] = &[
    pair("ocean towers / lavender farm", &OCEAN_TOWERS, &LAVENDER_SUNSET),
    pair("temple in rain / coral reef", &TEMPLE_RAIN, &CORAL_REEF),
    pair("starry dunes / maple garden", &DUNES_STARS, &MAPLE_GARDEN),
    pair("neon alley / mountain cabin", &NEON_ALLEY, &CABIN_SNOW_DAWN),
    pair("misty lake / Paris café", &MISTY_LAKE, &PARIS_CAFE),
    pair("coral reef / mountain cabin", &CORAL_REEF, &CABIN_SNOW_DAWN),
    pair("lighthouse storm / tulips and windmill", &LIGHTHOUSE_STORM, &TULIP_WINDMILL),
    pair("redwoods / Saturn", &REDWOODS, &SATURN_FROM_MOON),
    pair("greenhouse / polar bear", &GREENHOUSE, &ICE_FLOES),
    pair("kelp forest / sunflowers", &KELP_FOREST, &SUNFLOWERS),
];

// ── Measuring ─────────────────────────────────────────────────────────────────────────────────────

struct Measured {
    group: &'static str,
    label: &'static str,
    cosine: f32,
    a: Vec<f32>,
    b: Vec<f32>,
}

fn measure(embedder: &dyn Embedder, group: &'static str, pairs: &[Pair]) -> Vec<Measured> {
    pairs
        .iter()
        .map(|p| {
            let a = embedder.embed(&concept_text(&p.a.concept())).expect("embed a");
            let b = embedder.embed(&concept_text(&p.b.concept())).expect("embed b");
            Measured { group, label: p.label, cosine: cosine(&a, &b), a, b }
        })
        .collect()
}

fn stats(rows: &[Measured]) -> (f32, f32, f32) {
    let min = rows.iter().map(|m| m.cosine).fold(f32::INFINITY, f32::min);
    let max = rows.iter().map(|m| m.cosine).fold(f32::NEG_INFINITY, f32::max);
    let mean = rows.iter().map(|m| m.cosine).sum::<f32>() / rows.len() as f32;
    (min, max, mean)
}

/// Novelty of `b` against a memory holding only `a`, made today (bge-small embeddings).
fn novel_against(m: &Measured, policy: &NoveltyPolicy) -> bool {
    novel_against_model(m, policy, CandleEmbedder::MODEL_ID)
}

fn novel_against_model(m: &Measured, policy: &NoveltyPolicy, model_id: &str) -> bool {
    let now = 1_800_000_000;
    let memory = [MemoryRow {
        id: "a".into(),
        created_at: now,
        title: String::new(),
        summary: String::new(),
        embedding: m.a.clone(),
        embedding_model: model_id.into(),
        rating: Rating::Unrated,
        echo_of: None,
        mood_id: None,
    }];
    novelty::assess(&m.b, model_id, &memory, now, policy, None).is_novel(policy)
}

fn print_groups(groups: &[Vec<Measured>]) {
    println!("\n{:<16} {:<52} {:>6}", "group", "pair", "cosine");
    println!("{}", "─".repeat(76));
    for m in groups.iter().flatten() {
        println!("{:<16} {:<52} {:>6.3}", m.group, m.label, m.cosine);
    }
    println!("{}", "─".repeat(76));
    println!("{:<16} {:>6} {:>6} {:>6}", "group", "min", "mean", "max");
    for group in groups {
        let (min, max, mean) = stats(group);
        println!("{:<16} {min:>6.3} {mean:>6.3} {max:>6.3}", group[0].group);
    }
}

/// The fallback embedder (no model needed, so this always runs): the same pairs on its own scale. Stems
/// can't recognise a rewording, so near-duplicates are only measured; what it must get right is that
/// copies are repeats, related and unrelated ideas are novel, and echoes sit in its band.
#[test]
fn calibrates_novelty_against_the_hashing_fallback() {
    let embedder = HashingEmbedder;
    let groups = [
        measure(&embedder, "near-duplicate", NEAR_DUPLICATES),
        measure(&embedder, "echo", ECHOES),
        measure(&embedder, "related", RELATED),
        measure(&embedder, "unrelated", UNRELATED),
        measure(&embedder, "copy", COPIES),
    ];
    print_groups(&groups);
    let calibration = Calibration::for_model(HashingEmbedder::MODEL_ID);
    println!("HASHING_THRESHOLD = {HASHING_THRESHOLD}, HASHING_ECHO_BAND = {HASHING_ECHO_BAND:?}");
    let [near, echoes, related, unrelated, copies] = &groups;
    let caught = near.iter().filter(|m| m.cosine >= HASHING_THRESHOLD).count();
    println!("near-duplicates caught: {caught} of {} (a stem hash can't see rewording)\n", near.len());

    let policy = NoveltyPolicy { quiet_days: 182, threshold: HASHING_THRESHOLD };
    for m in copies {
        assert!(m.cosine >= HASHING_THRESHOLD, "copy judged novel: {} ({:.3})", m.label, m.cosine);
        assert!(!novel_against_model(m, &policy, HashingEmbedder::MODEL_ID), "copy passed novelty: {}", m.label);
        assert!(!calibration.in_echo_band(m.cosine), "copy accepted as an echo: {} ({:.3})", m.label, m.cosine);
    }
    for m in related.iter().chain(unrelated) {
        assert!(m.cosine < HASHING_THRESHOLD, "{} pair judged too similar: {} ({:.3})", m.group, m.label, m.cosine);
        assert!(novel_against_model(m, &policy, HashingEmbedder::MODEL_ID), "{} pair failed novelty", m.label);
    }
    for m in echoes {
        assert!(calibration.in_echo_band(m.cosine), "echo outside the band: {} ({:.3})", m.label, m.cosine);
    }
    for m in unrelated {
        assert!(m.cosine < HASHING_ECHO_BAND.0, "unrelated pair inside the band: {} ({:.3})", m.label, m.cosine);
    }
}

#[test]
#[ignore = "needs the real model (scripts/fetch-model.sh); run with --release -- --ignored --nocapture"]
fn calibrates_novelty_against_bge_small() {
    let embedder = load();
    let groups = [
        measure(&embedder, "near-duplicate", NEAR_DUPLICATES),
        measure(&embedder, "echo", ECHOES),
        measure(&embedder, "related", RELATED),
        measure(&embedder, "unrelated", UNRELATED),
        measure(&embedder, "copy", COPIES),
    ];

    print_groups(&groups);
    println!("DEFAULT_THRESHOLD = {DEFAULT_THRESHOLD}, ECHO_BAND = {ECHO_BAND:?}\n");

    let [near, echoes, related, unrelated, copies] = &groups;
    let in_band = near.iter().filter(|m| novelty::in_echo_band(m.cosine)).count();
    println!(
        "near-duplicates inside the echo band: {in_band} of {} (cosine can't tell them from echoes)\n",
        near.len()
    );

    let policy = NoveltyPolicy { quiet_days: 182, threshold: DEFAULT_THRESHOLD };
    for m in near {
        assert!(m.cosine >= DEFAULT_THRESHOLD, "near-duplicate judged novel: {} ({:.3})", m.label, m.cosine);
        assert!(!novel_against(m, &policy), "near-duplicate passed novelty: {}", m.label);
    }
    for m in echoes {
        assert!(novelty::in_echo_band(m.cosine), "echo outside the echo band: {} ({:.3})", m.label, m.cosine);
    }
    for m in related.iter().filter(|m| !KNOWN_RELATED_OVERLAPS.contains(&m.label)).chain(unrelated) {
        assert!(m.cosine < DEFAULT_THRESHOLD, "{} pair judged too similar: {} ({:.3})", m.group, m.label, m.cosine);
        assert!(novel_against(m, &policy), "{} pair failed novelty: {}", m.group, m.label);
    }
    for label in KNOWN_RELATED_OVERLAPS {
        let m = related.iter().find(|m| m.label == *label).expect("known overlaps name related pairs");
        assert!(
            m.cosine >= DEFAULT_THRESHOLD,
            "{label} ({:.3}) no longer overlaps the near-duplicates: drop it from KNOWN_RELATED_OVERLAPS",
            m.cosine
        );
    }
    for m in unrelated {
        assert!(m.cosine < ECHO_BAND.0, "unrelated pair inside the echo band: {} ({:.3})", m.label, m.cosine);
    }
    for m in copies {
        assert!(m.cosine >= ECHO_BAND.1, "copy accepted as an echo: {} ({:.3})", m.label, m.cosine);
    }
}

#[test]
#[ignore = "needs the real model (scripts/fetch-model.sh); run with --release -- --ignored --nocapture"]
fn embeddings_are_deterministic_unit_vectors() {
    let embedder = load();
    let text = concept_text(&OCEAN_TOWERS.concept());
    let a = embedder.embed(&text).expect("embed");
    let b = embedder.embed(&text).expect("embed");
    assert_eq!(a.len(), 384);
    assert_eq!(a, b);
    let norm = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 1e-5, "norm {norm}");

    // Past 512 tokens the text is truncated rather than rejected.
    let long = "rain over the old ruins ".repeat(400);
    let v = embedder.embed(&long).expect("embed long text");
    assert_eq!(v.len(), 384);
    // Empty text still embeds ([CLS] [SEP]).
    assert_eq!(embedder.embed("").expect("embed empty").len(), 384);
}
