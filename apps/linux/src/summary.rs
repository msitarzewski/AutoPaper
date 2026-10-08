//! "Your moods" (docs/app-spec.md, "Moods with nothing selected"): the summary of every mood, shown when the sidebar's
//! Moods row is chosen. From the top: the totals as stat tiles (moods, wallpapers, liked, echoes), the keywords by
//! weight in one line, a chart of the last 30 days' wallpapers stacked by mood, and every mood from most to least used,
//! each a card that opens the mood (its keywords by weight, Surprise, what it made, its latest wallpapers — a click
//! shows one on the desktop). Numbers come from the engine (`mood_stats`, `activity`), not from paging History.
//!
//! The chart (`ActivityChart`) follows the dataviz method: the validated categorical palette in a fixed order by when
//! each mood was made (reordering doesn't repaint), its own dark steps, thin bars with 4 px rounded data ends and a 2 px
//! surface gap between stacked segments, hairline gridlines, the scale on the trailing side, a legend for two moods or
//! more, a tooltip per day, and the same numbers as a table ("Show as table") — so no mood is told by colour alone, and
//! the colours that sit below 3:1 on the card have that relief. Past eight moods, and for deleted moods, bars fold
//! into "Other moods" (grey), never a generated colour.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use adw::prelude::*;
use autopaper_core::{DayCount, Mood, MoodStats};
use gtk::{gdk, glib, graphene, gsk};

use crate::app::{App, Event};
use crate::runtime::blocking;
use crate::strings;
use crate::wallpapers::{self, Item};

/// Days in the chart.
pub const DAYS: usize = 30;
/// Colours available (the palette's eight; a ninth series is "Other moods").
const SLOTS: usize = 8;
const OTHER_MOODS: &str = "Other moods";
/// The dataviz reference palette's categorical slots, light and dark steps (validated on libadwaita's card surfaces,
/// white and #2e2e32: every adjacent pair ≥ 8.4 ΔE for colour-blind readers, ≥ 19.3 for everyone else). The same as
/// the macOS app's `ChartPalette`, so a mood keeps its colour across apps.
const LIGHT: [u32; SLOTS] = [0x2A78D6, 0xEB6834, 0x1BAF7A, 0xEDA100, 0xE87BA4, 0x008300, 0x4A3AA7, 0xE34948];
const DARK: [u32; SLOTS] = [0x3987E5, 0xD95926, 0x199E70, 0xC98500, 0xD55181, 0x008300, 0x9085E9, 0xE66767];
const OTHER: u32 = 0x8A8984;
const THUMB_WIDTH: i32 = 96;
const THUMB_HEIGHT: i32 = 60;

// ── The numbers behind the chart ────────────────────────────────────────────────────────────────────────────────

/// The `day_bounds` for `activity`: the local midnight starting each of the last `days` days (today last), then
/// tomorrow's, so a 23- or 25-hour day (daylight saving) is still one day.
pub fn day_bounds(days: usize, now: &glib::DateTime) -> Vec<i64> {
    // Noon of each day, so stepping a day at a time never lands on the wrong date across a clock change.
    let (year, month, day) = now.ymd();
    let Ok(noon) = glib::DateTime::from_local(year, month, day, 12, 0, 0.0) else { return Vec::new() };
    (0..=days as i32)
        .filter_map(|offset| {
            let date = noon.add_days(offset - days as i32 + 1).ok()?;
            let (y, m, d) = date.ymd();
            glib::DateTime::from_local(y, m, d, 0, 0, 0.0).ok().map(|midnight| midnight.to_unix())
        })
        .collect()
}

/// Each mood's colour slot (0–7), by when it was made (oldest first; then id). With more than eight moods the newest
/// share "Other moods" (no slot), so no colour is ever generated past the palette.
pub fn color_slots(moods: &[Mood]) -> HashMap<String, usize> {
    let mut by_age: Vec<&Mood> = moods.iter().collect();
    by_age.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
    let coloured = if by_age.len() > SLOTS { SLOTS - 1 } else { by_age.len() };
    by_age.into_iter().take(coloured).enumerate().map(|(slot, mood)| (mood.id.clone(), slot)).collect()
}

/// One stacked segment: a mood (or "Other moods") and how many it made that day.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// The mood's colour slot; `None` for "Other moods".
    pub slot: Option<usize>,
    pub name: String,
    pub count: u32,
}

/// The engine's counts as one stack per day (`bounds.len() - 1` days, oldest first), each day's segments in the moods'
/// list order with "Other moods" on top.
pub fn stacks(counts: &[DayCount], moods: &[Mood], slots: &HashMap<String, usize>, bounds: &[i64]) -> Vec<Vec<Segment>> {
    let days = bounds.len().saturating_sub(1);
    let mut result: Vec<Vec<Segment>> = vec![Vec::new(); days];
    for count in counts {
        let Some(day) = bounds.iter().take(days).position(|start| *start == count.day_start) else { continue };
        let mood = count.mood_id.as_ref().and_then(|id| moods.iter().find(|mood| mood.id == *id));
        let (slot, name) = match mood.and_then(|mood| slots.get(&mood.id).map(|slot| (mood, *slot))) {
            Some((mood, slot)) => (Some(slot), mood.name.clone()),
            None => (None, OTHER_MOODS.to_string()),
        };
        let stack = &mut result[day];
        match stack.iter_mut().find(|segment| segment.slot == slot && segment.name == name) {
            Some(segment) => segment.count += count.count,
            None => stack.push(Segment { slot, name, count: count.count }),
        }
    }
    let place = |segment: &Segment| -> usize {
        match segment.slot {
            None => usize::MAX,
            Some(_) => moods.iter().find(|mood| mood.name == segment.name).map(|mood| mood.position as usize).unwrap_or(usize::MAX - 1),
        }
    };
    for stack in &mut result {
        stack.sort_by_key(place);
    }
    result
}

/// The y-axis: whole numbers from 0 to a top at or above `maximum`, at most five ticks (0, 1, 2 · 0, 2, 4, 6 · 0, 5,
/// 10, 15…).
pub fn ticks(maximum: u32) -> Vec<u32> {
    let top = maximum.max(1);
    let step = [1, 2, 5, 10, 20, 25, 50, 100, 200, 250, 500, 1_000]
        .into_iter()
        .find(|step| top.div_ceil(*step) <= 4)
        .unwrap_or_else(|| (top / 4).max(1));
    let last = top.div_ceil(step) * step;
    (0..=last).step_by(step as usize).collect()
}

/// Which days the axis names: a week apart, ending five days before the last, so the last label sits clear of the
/// scale on the trailing side.
pub fn week_labels(days: usize) -> Vec<usize> {
    if days < 7 {
        return Vec::new();
    }
    let mut labels: Vec<usize> = (0..=days - 6).rev().step_by(7).collect();
    labels.reverse();
    labels
}

fn rgba(hex: u32) -> gdk::RGBA {
    gdk::RGBA::new(((hex >> 16) & 0xFF) as f32 / 255.0, ((hex >> 8) & 0xFF) as f32 / 255.0, (hex & 0xFF) as f32 / 255.0, 1.0)
}

fn series_color(slot: Option<usize>, dark: bool) -> gdk::RGBA {
    match slot {
        Some(slot) if slot < SLOTS => rgba(if dark { DARK[slot] } else { LIGHT[slot] }),
        _ => rgba(OTHER),
    }
}

// ── The chart widget ────────────────────────────────────────────────────────────────────────────────────────────

/// What the chart draws.
#[derive(Default, Clone)]
pub struct ChartData {
    pub stacks: Vec<Vec<Segment>>,
    /// Each day's start (`stacks`' days), for the axis and the tooltip.
    pub starts: Vec<i64>,
    /// Console groups runs by UTC day; wallpaper activity uses local calendar days.
    pub utc: bool,
}

mod imp {
    use super::*;
    use gtk::subclass::prelude::*;

    #[derive(Default)]
    pub struct ActivityChart {
        pub data: RefCell<ChartData>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ActivityChart {
        const NAME: &'static str = "AutoPaperActivityChart";
        type Type = super::ActivityChart;
        type ParentType = gtk::Widget;

        fn class_init(class: &mut Self::Class) {
            class.set_accessible_role(gtk::AccessibleRole::Img);
            class.set_css_name("activitychart");
        }
    }

    impl ObjectImpl for ActivityChart {
        fn constructed(&self) {
            self.parent_constructed();
            let widget = self.obj();
            widget.set_has_tooltip(true);
            widget.connect_query_tooltip(|widget, x, _, _, tooltip| match widget.day_at(f64::from(x)) {
                Some(day) => {
                    tooltip.set_text(Some(&widget.day_line(day)));
                    true
                }
                None => false,
            });
            // Its own colours for the dark style (not a flip of the light ones).
            let weak = widget.downgrade();
            adw::StyleManager::default().connect_dark_notify(move |_| {
                if let Some(widget) = weak.upgrade() {
                    widget.queue_draw();
                }
            });
        }
    }

    impl WidgetImpl for ActivityChart {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            match orientation {
                gtk::Orientation::Horizontal => (220, 600, -1, -1),
                _ => (190, 190, -1, -1),
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            self.obj().draw(snapshot);
        }
    }
}

glib::wrapper! {
    /// "Wallpapers, last 30 days": a bar per day, stacked by mood, the scale on the trailing side.
    pub struct ActivityChart(ObjectSubclass<imp::ActivityChart>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// Room for the date labels under the bars, and for the scale beside them.
const AXIS_HEIGHT: f32 = 22.0;
const SCALE_WIDTH: f32 = 30.0;
const TOP: f32 = 8.0;

impl ActivityChart {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn set_data(&self, data: ChartData) {
        use gtk::subclass::prelude::ObjectSubclassIsExt;
        self.imp().data.replace(data);
        self.queue_draw();
    }

    fn data(&self) -> ChartData {
        use gtk::subclass::prelude::ObjectSubclassIsExt;
        self.imp().data.borrow().clone()
    }

    /// The plot's left edge, width and each day's band width.
    fn geometry(&self) -> (f32, f32, f32) {
        let days = self.data().stacks.len().max(1) as f32;
        let plot = (self.width() as f32 - SCALE_WIDTH).max(1.0);
        (0.0, plot, plot / days)
    }

    /// The day under `x`, if it has any wallpapers.
    fn day_at(&self, x: f64) -> Option<usize> {
        let (left, plot, band) = self.geometry();
        let x = x as f32 - left;
        if x < 0.0 || x >= plot {
            return None;
        }
        let day = (x / band) as usize;
        self.data().stacks.get(day).filter(|stack| !stack.is_empty()).map(|_| day)
    }

    /// "Monday 5 October: Rainy beach 2, Night city 1".
    fn day_line(&self, day: usize) -> String {
        let data = self.data();
        let date = data.starts.get(day).and_then(|start| chart_date(*start, data.utc).ok());
        let date = date.and_then(|date| date.format("%A %-e %B").ok()).map(|text| text.to_string()).unwrap_or_default();
        let stack = data.stacks.get(day).cloned().unwrap_or_default();
        format!("{date}: {}", day_parts(&stack))
    }

    fn draw(&self, snapshot: &gtk::Snapshot) {
        let data = self.data();
        let width = self.width() as f32;
        let height = self.height() as f32;
        if data.stacks.is_empty() || width <= SCALE_WIDTH {
            return;
        }
        let dark = adw::StyleManager::default().is_dark();
        let ink = self.color();
        let mut muted = ink;
        muted.set_alpha(0.8);
        let mut grid = ink;
        grid.set_alpha(0.12);
        let totals: Vec<u32> = data.stacks.iter().map(|stack| stack.iter().map(|segment| segment.count).sum()).collect();
        let ticks = ticks(totals.iter().copied().max().unwrap_or(0));
        let top_value = (*ticks.last().unwrap_or(&1)).max(1) as f32;
        let (left, plot, band) = self.geometry();
        let base = height - AXIS_HEIGHT;
        let scale = (base - TOP) / top_value;

        // Hairline gridlines, solid; the numbers on the trailing side in secondary ink.
        for tick in &ticks {
            let y = (base - *tick as f32 * scale).round();
            snapshot.append_color(&grid, &graphene::Rect::new(left, y, plot, 1.0));
            let layout = self.create_pango_layout(Some(&tick.to_string()));
            let (_, text_height) = layout.pixel_size();
            snapshot.save();
            snapshot.translate(&graphene::Point::new(left + plot + 6.0, y - text_height as f32 / 2.0));
            snapshot.append_layout(&layout, &muted);
            snapshot.restore();
        }

        // Thin bars (never filling the band), stacked bottom-up with a 2 px gap; the top one's data end rounded.
        let bar = (band * 0.6).clamp(2.0, 24.0);
        for (day, stack) in data.stacks.iter().enumerate() {
            let x = left + day as f32 * band + (band - bar) / 2.0;
            let mut y = base;
            let last = stack.len().saturating_sub(1);
            for (index, segment) in stack.iter().enumerate() {
                let tall = segment.count as f32 * scale;
                // The gap comes out of the segment above the one below (it never moves the baseline or the top).
                let gap = if index > 0 { 2.0 } else { 0.0 };
                let rect = graphene::Rect::new(x, y - tall, bar, (tall - gap).max(1.0));
                let color = series_color(segment.slot, dark);
                if index == last {
                    let radius = graphene::Size::new(4.0_f32.min(bar / 2.0), 4.0_f32.min(rect.height()));
                    let zero = graphene::Size::new(0.0, 0.0);
                    snapshot.push_rounded_clip(&gsk::RoundedRect::new(rect, radius, radius, zero, zero));
                    snapshot.append_color(&color, &rect);
                    snapshot.pop();
                } else {
                    snapshot.append_color(&color, &rect);
                }
                y -= tall;
            }
        }

        // The day axis: a label a week apart.
        let labels = if data.utc && data.stacks.len() < 7 {
            if data.stacks.len() > 1 { vec![0, data.stacks.len() - 1] } else { vec![0] }
        } else {
            week_labels(data.stacks.len())
        };
        for day in labels {
            let Some(start) = data.starts.get(day) else { continue };
            let Ok(date) = chart_date(*start, data.utc) else { continue };
            let Ok(text) = date.format("%-e %b") else { continue };
            let layout = self.create_pango_layout(Some(text.as_str()));
            let (text_width, _) = layout.pixel_size();
            let centre = left + day as f32 * band + band / 2.0;
            let x = (centre - text_width as f32 / 2.0).clamp(left, (left + plot - text_width as f32).max(left));
            snapshot.save();
            snapshot.translate(&graphene::Point::new(x, base + 4.0));
            snapshot.append_layout(&layout, &muted);
            snapshot.restore();
        }
    }
}

impl Default for ActivityChart {
    fn default() -> Self {
        Self::new()
    }
}

fn chart_date(unix: i64, utc: bool) -> Result<glib::DateTime, glib::BoolError> {
    if utc { glib::DateTime::from_unix_utc(unix) } else { glib::DateTime::from_unix_local(unix) }
}

/// One day's segments in words: "Rainy beach 2, Night city 1" (or "no wallpapers").
fn day_parts(stack: &[Segment]) -> String {
    if stack.is_empty() {
        return "no wallpapers".into();
    }
    stack.iter().map(|segment| format!("{} {}", segment.name, segment.count)).collect::<Vec<_>>().join(", ")
}

// ── Stat tiles ──────────────────────────────────────────────────────────────────────────────────────────────────

/// A number with what it counts, on a card: a small icon and label above, the number below. Screen readers read the
/// label, then the number; the tile's name says both ("12 liked").
pub struct StatTile {
    root: gtk::Box,
    value: gtk::Label,
}

impl StatTile {
    pub fn new(icon: &str, title: &str, compact: bool) -> Self {
        let caption = gtk::Box::builder().spacing(6).css_classes(["secondary-text"]).build();
        caption.append(&crate::history::decorative_icon(icon));
        caption.append(&gtk::Label::builder().label(title).xalign(0.0).css_classes(["caption"]).build());
        let value = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes([if compact { "title-3" } else { "title-2" }])
            .build();
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(if compact { 2 } else { 6 })
            .hexpand(true)
            .width_request(if compact { 110 } else { 136 })
            .css_classes(["card", "stat-tile"])
            .accessible_role(gtk::AccessibleRole::Group)
            .build();
        if compact {
            root.add_css_class("compact");
        }
        root.append(&caption);
        root.append(&value);
        Self { root, value }
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// The number shown and the tile as spoken ("12 liked").
    pub fn set(&self, value: &str, spoken: &str) {
        self.value.set_label(value);
        self.root.update_property(&[gtk::accessible::Property::Label(spoken)]);
    }
}

// ── The page ────────────────────────────────────────────────────────────────────────────────────────────────────

pub struct SummaryPage {
    root: adw::ToolbarView,
    app: Weak<App>,
    tiles: [StatTile; 4],
    keywords: gtk::Label,
    chart: ActivityChart,
    chart_empty: gtk::Label,
    legend: adw::WrapBox,
    footnote: gtk::Label,
    table_expander: gtk::Expander,
    table: gtk::Box,
    cards: gtk::Box,
    /// Shown, not read yet: refreshed when the page is next shown.
    stale: Cell<bool>,
    /// Bumped on every refresh, so an activity read that arrives after a newer one is dropped.
    epoch: Cell<u64>,
}

impl SummaryPage {
    pub fn new(app: &Rc<App>) -> Rc<Self> {
        let title = gtk::Label::builder()
            .label("Your moods")
            .xalign(0.0)
            .css_classes(["title-1"])
            .accessible_role(gtk::AccessibleRole::Heading)
            .build();
        let tiles = [
            StatTile::new("view-list-bullet-symbolic", "Moods", false),
            StatTile::new("image-x-generic-symbolic", "Wallpapers", false),
            StatTile::new("autopaper-like-symbolic", "Liked", false),
            StatTile::new("media-playlist-repeat-symbolic", "Echoes", false),
        ];
        let tile_box = adw::WrapBox::builder().child_spacing(12).line_spacing(12).justify(adw::JustifyMode::Fill).build();
        for tile in &tiles {
            tile_box.append(tile.widget());
        }
        let keywords = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["secondary-text"]).build();

        let chart_title = heading("Wallpapers, last 30 days");
        let chart = ActivityChart::new();
        chart.update_property(&[gtk::accessible::Property::Label("Wallpapers, last 30 days")]);
        let chart_empty = gtk::Label::builder()
            .label(format!("No wallpapers in the last {DAYS} days."))
            .xalign(0.0)
            .css_classes(["secondary-text"])
            .visible(false)
            .build();
        let legend = adw::WrapBox::builder().child_spacing(16).line_spacing(4).build();
        legend.update_property(&[gtk::accessible::Property::Label("Legend")]);
        let footnote = gtk::Label::builder().xalign(0.0).wrap(true).css_classes(["secondary-text", "caption"]).build();
        let table = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .margin_top(6)
            .accessible_role(gtk::AccessibleRole::Table)
            .build();
        table.update_property(&[gtk::accessible::Property::Label("Wallpapers, last 30 days")]);
        let table_expander = gtk::Expander::builder().label("Show as table").child(&table).build();
        let chart_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        chart_box.append(&chart);
        chart_box.append(&chart_empty);
        chart_box.append(&legend);
        chart_box.append(&footnote);
        chart_box.append(&table_expander);
        let chart_card = gtk::Box::builder().css_classes(["card"]).build();
        chart_card.append(&chart_box);

        let cards = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).build();
        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(24)
            .margin_bottom(24)
            .margin_start(12)
            .margin_end(12)
            .build();
        column.append(&title);
        column.append(&tile_box);
        column.append(&keywords);
        let spacer = |widget: &gtk::Widget| widget.set_margin_top(12);
        spacer(chart_title.upcast_ref());
        column.append(&chart_title);
        column.append(&chart_card);
        let most_used = heading("Most used");
        spacer(most_used.upcast_ref());
        column.append(&most_used);
        column.append(&cards);
        let clamp = adw::Clamp::builder().maximum_size(860).child(&column).build();
        let scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&clamp).build();
        let header = adw::HeaderBar::new();
        header.pack_end(&crate::window::progress_button(app));
        let root = adw::ToolbarView::new();
        root.add_top_bar(&header);
        root.set_content(Some(&scroller));

        let this = Rc::new(Self {
            root,
            app: Rc::downgrade(app),
            tiles,
            keywords,
            chart,
            chart_empty,
            legend,
            footnote,
            table_expander,
            table,
            cards,
            stale: Cell::new(true),
            epoch: Cell::new(0),
        });
        let weak = Rc::downgrade(&this);
        this.root.connect_map(move |_| {
            if let Some(this) = weak.upgrade()
                && this.stale.get()
            {
                this.refresh();
            }
        });
        let weak = Rc::downgrade(&this);
        app.subscribe(move |event| {
            let Some(this) = weak.upgrade() else { return false };
            if matches!(event, Event::Ready | Event::Moods | Event::MoodStats | Event::History) {
                if this.root.is_mapped() {
                    this.refresh();
                } else {
                    this.stale.set(true);
                }
            }
            true
        });
        this
    }

    pub fn widget(&self) -> &adw::ToolbarView {
        &self.root
    }

    fn refresh(self: &Rc<Self>) {
        let Some(app) = self.app.upgrade() else { return };
        let Some(engine) = app.engine() else { return };
        self.stale.set(false);
        self.epoch.set(self.epoch.get() + 1);
        let epoch = self.epoch.get();
        let (moods, stats) = {
            let state = app.state();
            (state.moods.clone(), state.mood_stats.clone())
        };
        self.show_totals(&moods, &stats);
        let Ok(now) = glib::DateTime::now_local() else { return };
        let bounds = day_bounds(DAYS, &now);
        let weak = Rc::downgrade(self);
        let latest: Vec<autopaper_core::Generation> = stats.iter().flat_map(|stats| stats.latest.clone()).collect();
        glib::spawn_future_local(async move {
            let asked = bounds.clone();
            let read = blocking(move || {
                let counts = engine.activity(asked)?;
                let items: HashMap<String, Item> = latest
                    .into_iter()
                    .map(|generation| (generation.id.clone(), Item::read(&engine, generation)))
                    .collect();
                Ok((counts, items))
            })
            .await;
            let Some(this) = weak.upgrade() else { return };
            if this.epoch.get() != epoch {
                return;
            }
            match read {
                Ok((counts, items)) => {
                    this.show_chart(&moods, &counts, &bounds);
                    this.show_cards(&app, &moods, &stats, &items);
                }
                Err(error) => tracing::warn!(%error, "couldn't read the moods' activity"),
            }
        });
    }

    fn show_totals(&self, moods: &[Mood], stats: &[MoodStats]) {
        let made: u32 = stats.iter().map(|stats| stats.wallpapers).sum();
        let liked: u32 = stats.iter().map(|stats| stats.liked).sum();
        let echoes: u32 = stats.iter().map(|stats| stats.echoes).sum();
        let count = moods.len() as u32;
        self.tiles[0].set(&count.to_string(), &strings::count(count, "mood", "moods"));
        self.tiles[1].set(&made.to_string(), &strings::count(made, "wallpaper made", "wallpapers made"));
        self.tiles[2].set(&liked.to_string(), &format!("{liked} liked"));
        self.tiles[3].set(&echoes.to_string(), &strings::count(echoes, "echo", "echoes"));
        self.keywords.set_label(&strings::keyword_breakdown(moods));
    }

    fn show_chart(&self, moods: &[Mood], counts: &[DayCount], bounds: &[i64]) {
        let slots = color_slots(moods);
        let stacks = stacks(counts, moods, &slots, bounds);
        let total: u32 = stacks.iter().flatten().map(|segment| segment.count).sum();
        let starts: Vec<i64> = bounds.iter().take(stacks.len()).copied().collect();
        let empty = total == 0;
        self.chart.set_visible(!empty);
        self.chart_empty.set_visible(empty);
        self.footnote.set_visible(!empty);
        self.table_expander.set_visible(!empty);
        // The moods (and "Other moods") that have bars, in the order they stack.
        let mut series: Vec<(Option<usize>, String)> = Vec::new();
        for segment in stacks.iter().flatten() {
            if !series.iter().any(|(slot, name)| *slot == segment.slot && *name == segment.name) {
                series.push((segment.slot, segment.name.clone()));
            }
        }
        series.sort_by_key(|(slot, name)| match slot {
            None => usize::MAX,
            Some(_) => moods.iter().find(|mood| mood.name == *name).map(|mood| mood.position as usize).unwrap_or(usize::MAX - 1),
        });
        // A legend for two series or more (one is named by the title).
        while let Some(child) = self.legend.first_child() {
            self.legend.remove(&child);
        }
        self.legend.set_visible(series.len() >= 2);
        for (slot, name) in &series {
            let key = gtk::Box::builder().spacing(6).build();
            let dot = gtk::Box::builder()
                .width_request(10)
                .height_request(10)
                .valign(gtk::Align::Center)
                .css_classes(["chart-dot", &slot.map(|slot| format!("slot-{slot}")).unwrap_or_else(|| "other".into())])
                .accessible_role(gtk::AccessibleRole::Presentation)
                .build();
            key.append(&dot);
            key.append(&gtk::Label::new(Some(name)));
            self.legend.append(&key);
        }
        let busiest = stacks
            .iter()
            .enumerate()
            .map(|(day, stack)| (day, stack.iter().map(|segment| segment.count).sum::<u32>()))
            .max_by_key(|(day, total)| (*total, *day));
        let made = strings::count(total, "wallpaper", "wallpapers");
        self.footnote.set_label(&format!("{made} in the last {DAYS} days. Kept on this computer only."));
        let summary = match busiest {
            Some((day, most)) if most > 0 => {
                let date = starts
                    .get(day)
                    .and_then(|start| glib::DateTime::from_unix_local(*start).ok())
                    .and_then(|date| date.format("%-e %B").ok())
                    .map(|text| text.to_string())
                    .unwrap_or_default();
                format!("{made} in the last {DAYS} days, most on {date} ({most}). Show as table lists every day.")
            }
            _ => format!("No wallpapers in the last {DAYS} days."),
        };
        self.chart.update_property(&[gtk::accessible::Property::Description(&summary)]);
        self.fill_table(&stacks, &starts);
        self.chart.set_data(ChartData { stacks, starts, utc: false });
    }

    /// The chart's numbers as a table: a row per day with wallpapers (Day · Moods · Wallpapers).
    fn fill_table(&self, stacks: &[Vec<Segment>], starts: &[i64]) {
        while let Some(child) = self.table.first_child() {
            self.table.remove(&child);
        }
        let row = |cells: [String; 3], header: bool| {
            let row = gtk::Box::builder().spacing(12).accessible_role(gtk::AccessibleRole::Row).build();
            for (index, text) in cells.into_iter().enumerate() {
                let cell = gtk::Label::builder()
                    .label(text)
                    .xalign(if index == 2 { 1.0 } else { 0.0 })
                    .wrap(true)
                    .hexpand(index == 1)
                    .width_request(if index == 0 { 140 } else { -1 })
                    .accessible_role(if header { gtk::AccessibleRole::ColumnHeader } else { gtk::AccessibleRole::Cell })
                    .build();
                if header {
                    cell.add_css_class("heading");
                }
                row.append(&cell);
            }
            row
        };
        self.table.append(&row(["Day".into(), "Moods".into(), "Wallpapers".into()], true));
        for (day, stack) in stacks.iter().enumerate().rev().filter(|(_, stack)| !stack.is_empty()) {
            let date = starts
                .get(day)
                .and_then(|start| glib::DateTime::from_unix_local(*start).ok())
                .and_then(|date| date.format("%a %-e %b").ok())
                .map(|text| text.to_string())
                .unwrap_or_default();
            let total: u32 = stack.iter().map(|segment| segment.count).sum();
            self.table.append(&row([date, day_parts(stack), total.to_string()], false));
        }
    }

    /// Every mood, most used first (equal ones in the person's order), each a card.
    fn show_cards(&self, app: &Rc<App>, moods: &[Mood], stats: &[MoodStats], items: &HashMap<String, Item>) {
        // Keyboard focus on a card's name stays on that mood's card after the cards are rebuilt.
        let focused = self.root.root().and_then(|root| gtk::prelude::RootExt::focus(&root)).and_then(|focus| {
            let mut child = self.cards.first_child();
            while let Some(card) = child {
                if focus.is_ancestor(&card) {
                    return Some(card.widget_name().to_string());
                }
                child = card.next_sibling();
            }
            None
        });
        while let Some(child) = self.cards.first_child() {
            self.cards.remove(&child);
        }
        let slots = color_slots(moods);
        let made = |mood: &Mood| stats.iter().find(|stats| stats.mood_id == mood.id).map(|stats| stats.wallpapers).unwrap_or(0);
        let mut ordered: Vec<&Mood> = moods.iter().collect();
        ordered.sort_by_key(|mood| (std::cmp::Reverse(made(mood)), mood.position));
        let now = glib::real_time() / 1_000_000;
        for mood in ordered {
            let stats = stats.iter().find(|stats| stats.mood_id == mood.id);
            let card = mood_card(app, mood, stats, slots.get(&mood.id).copied(), items, now);
            self.cards.append(&card);
            if focused.as_deref() == Some(mood.id.as_str())
                && let Some(name) = card.first_child().and_then(|header| header.first_child()).and_then(|dot| dot.next_sibling())
            {
                name.grab_focus();
            }
        }
    }
}

fn heading(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).css_classes(["title-4"]).accessible_role(gtk::AccessibleRole::Heading).build()
}

/// One mood in the summary: a dot in its chart colour, its name (opens it), "Current mood" or Use, how many wallpapers
/// it made, its keywords by weight, its Surprise, liked and last made, and its latest wallpapers. A click anywhere
/// else on the card opens the mood too.
fn mood_card(
    app: &Rc<App>,
    mood: &Mood,
    stats: Option<&MoodStats>,
    slot: Option<usize>,
    items: &HashMap<String, Item>,
    now: i64,
) -> gtk::Box {
    let made = stats.map(|stats| stats.wallpapers).unwrap_or(0);
    let liked = stats.map(|stats| stats.liked).unwrap_or(0);
    let last = stats.and_then(|stats| stats.last_made_at);

    let dot = gtk::Box::builder()
        .width_request(10)
        .height_request(10)
        .valign(gtk::Align::Center)
        .css_classes(["chart-dot", &slot.map(|slot| format!("slot-{slot}")).unwrap_or_else(|| "other".into())])
        .accessible_role(gtk::AccessibleRole::Presentation)
        .build();
    let name_content = gtk::Box::builder().spacing(4).build();
    name_content.append(&gtk::Label::builder().label(&mood.name).css_classes(["heading"]).ellipsize(gtk::pango::EllipsizeMode::End).build());
    name_content.append(&crate::history::decorative_icon("go-next-symbolic"));
    let name = gtk::Button::builder().child(&name_content).css_classes(["flat", "mood-card-name"]).build();
    name.set_tooltip_text(Some(&format!("Show {}", mood.name)));
    name.update_property(&[
        gtk::accessible::Property::Label(&strings::mood_spoken(mood)),
        gtk::accessible::Property::Description("Shows its keywords and Surprise"),
    ]);
    let id = mood.id.clone();
    let weak = Rc::downgrade(app);
    let open = move || {
        if let Some(window) = weak.upgrade().and_then(|app| app.main_window()) {
            window.show_mood(&id);
        }
    };
    let open_name = open.clone();
    name.connect_clicked(move |_| open_name());

    let header = gtk::Box::builder().spacing(8).build();
    header.append(&dot);
    header.append(&name);
    if mood.active {
        // Said with the name ("Rainy beach, current mood").
        let current = gtk::Box::builder().spacing(4).valign(gtk::Align::Center).accessible_role(gtk::AccessibleRole::Presentation).build();
        current.append(&crate::history::decorative_icon("object-select-symbolic"));
        current.append(&gtk::Label::builder().label("Current mood").accessible_role(gtk::AccessibleRole::Presentation).build());
        header.append(&current);
    } else {
        let use_button = gtk::Button::builder().label("Use").valign(gtk::Align::Center).build();
        // Starts with the visible label (WCAG 2.5.3), so "click Use" finds it.
        use_button.update_property(&[gtk::accessible::Property::Label(&format!("Use {}", mood.name))]);
        use_button.set_tooltip_text(Some("Make this the current mood. Nothing changes until the next wallpaper."));
        let (weak, id) = (Rc::downgrade(app), mood.id.clone());
        use_button.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.use_mood(id.clone());
            }
        });
        header.append(&use_button);
    }
    let count = gtk::Box::builder().orientation(gtk::Orientation::Vertical).halign(gtk::Align::End).hexpand(true).build();
    count.append(&gtk::Label::builder().label(made.to_string()).xalign(1.0).css_classes(["title-3"]).build());
    count.append(
        &gtk::Label::builder()
            .label(if made == 1 { "wallpaper" } else { "wallpapers" })
            .xalign(1.0)
            .css_classes(["secondary-text", "caption"])
            .build(),
    );
    count.set_accessible_role(gtk::AccessibleRole::Group);
    count.update_property(&[gtk::accessible::Property::Label(&strings::count(made, "wallpaper", "wallpapers"))]);
    header.append(&count);

    let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(3).build();
    let groups = strings::keyword_groups(mood.keywords.iter().map(|keyword| (keyword.text.as_str(), keyword.weight)));
    if groups.is_empty() {
        body.append(&gtk::Label::builder().label("No keywords yet").xalign(0.0).build());
    }
    for group in groups {
        body.append(&gtk::Label::builder().label(&group).xalign(0.0).wrap(true).build());
    }
    let line = format!("{} · {}", strings::surprise_line(mood.surprise), strings::made_line(made, liked, last, " · ", now));
    let spoken = format!("{}, {}", strings::surprise_line(mood.surprise), strings::made_line(made, liked, last, ", ", now));
    let line = gtk::Label::builder().label(&line).xalign(0.0).wrap(true).css_classes(["secondary-text"]).build();
    line.update_property(&[gtk::accessible::Property::Label(&spoken)]);
    body.append(&line);

    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .css_classes(["card", "mood-card"])
        .name(&mood.id)
        .build();
    card.append(&header);
    card.append(&body);
    let latest: Vec<Item> = stats
        .map(|stats| stats.latest.iter().filter_map(|generation| items.get(&generation.id).cloned()).collect())
        .unwrap_or_default();
    if !latest.is_empty() {
        let titles: Vec<String> = latest.iter().map(|item| item.generation.concept.title.clone()).collect();
        let strip = wallpapers::thumbnail_strip(app, &latest, THUMB_WIDTH, THUMB_HEIGHT, &strings::spoken_latest(&titles));
        card.append(&strip);
    }
    // A click anywhere else on the card opens the mood (the buttons on it keep their own clicks).
    let click = gtk::GestureClick::new();
    click.connect_released(move |gesture, presses, _, _| {
        if presses == 1 {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            open();
        }
    });
    card.add_controller(click);
    card
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mood(id: &str, position: u32, created_at: i64) -> Mood {
        Mood {
            id: id.into(),
            name: id.to_uppercase(),
            position,
            surprise: 0.35,
            created_at,
            keywords: Vec::new(),
            active: position == 0,
        }
    }

    #[test]
    fn ticks_are_whole_and_few() {
        assert_eq!(ticks(0), vec![0, 1]);
        assert_eq!(ticks(3), vec![0, 1, 2, 3]);
        assert_eq!(ticks(6), vec![0, 2, 4, 6]);
        assert_eq!(ticks(14), vec![0, 5, 10, 15]);
        assert!(ticks(999).len() <= 5);
    }

    #[test]
    fn day_bounds_are_local_midnights_ending_tomorrow() {
        let now = glib::DateTime::from_local(2026, 10, 6, 15, 30, 0.0).unwrap();
        let bounds = day_bounds(30, &now);
        assert_eq!(bounds.len(), 31);
        assert!(bounds.windows(2).all(|pair| pair[0] < pair[1]));
        let last = glib::DateTime::from_unix_local(bounds[30]).unwrap();
        assert_eq!((last.ymd(), last.hour()), ((2026, 10, 7), 0));
        let first = glib::DateTime::from_unix_local(bounds[0]).unwrap();
        assert_eq!(first.ymd(), (2026, 9, 7));
    }

    #[test]
    fn colours_follow_the_mood_not_its_place() {
        let moods = vec![mood("b", 0, 20), mood("a", 1, 10)];
        let slots = color_slots(&moods);
        // The older mood has the first colour wherever it sits in the list.
        assert_eq!(slots["a"], 0);
        assert_eq!(slots["b"], 1);
        // Past eight, the newest fold into "Other moods".
        let many: Vec<Mood> = (0..10).map(|i| mood(&format!("m{i}"), i, i64::from(i))).collect();
        let slots = color_slots(&many);
        assert_eq!(slots.len(), 7);
        assert!(!slots.contains_key("m7") && !slots.contains_key("m9"));
    }

    #[test]
    fn stacks_follow_the_moods_order_with_others_on_top() {
        let moods = vec![mood("x", 0, 2), mood("y", 1, 1)];
        let slots = color_slots(&moods);
        let bounds = [100, 200, 300];
        let counts = [
            DayCount { day_start: 200, mood_id: None, count: 1 },
            DayCount { day_start: 200, mood_id: Some("y".into()), count: 2 },
            DayCount { day_start: 200, mood_id: Some("x".into()), count: 3 },
            DayCount { day_start: 200, mood_id: Some("gone".into()), count: 1 },
        ];
        let stacks = stacks(&counts, &moods, &slots, &bounds);
        assert_eq!(stacks.len(), 2);
        assert!(stacks[0].is_empty());
        let names: Vec<(&str, u32)> = stacks[1].iter().map(|segment| (segment.name.as_str(), segment.count)).collect();
        assert_eq!(names, vec![("X", 3), ("Y", 2), (OTHER_MOODS, 2)]);
        assert_eq!(day_parts(&stacks[1]), "X 3, Y 2, Other moods 2");
    }

    #[test]
    fn week_labels_end_clear_of_the_scale() {
        assert_eq!(week_labels(30), vec![3, 10, 17, 24]);
        assert!(week_labels(5).is_empty());
    }
}
