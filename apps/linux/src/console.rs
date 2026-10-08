//! Local generation runs, including attempts that never made an image. Dates expand to outcomes; the selected
//! run shows every retained request, response and retry. Engine reads stay off GTK's main thread.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use adw::prelude::*;
use autopaper_core::{ConsoleStatistics, ProviderJob, RunRecord, RunStatus};
use gtk::{gio, glib};

use crate::app::{App, Event};
use crate::runtime::blocking;
use crate::strings;

const PAGE_SIZE: u32 = 30;
const OUTCOMES: [RunStatus; 6] = [
    RunStatus::Succeeded,
    RunStatus::Failed,
    RunStatus::Blocked,
    RunStatus::Cancelled,
    RunStatus::Interrupted,
    RunStatus::Running,
];

pub struct ConsolePage {
    root: adw::ToolbarView,
    app: Weak<App>,
    panes: gtk::Paned,
    list_stack: gtk::Stack,
    detail_stack: gtk::Stack,
    statistics: gtk::Expander,
    totals: gtk::Label,
    outcomes: gtk::Box,
    activity: crate::summary::ActivityChart,
    daily_legend: adw::WrapBox,
    daily_table: gtk::Box,
    model_times: gtk::Box,
    dates: gtk::ListBox,
    date_rows: RefCell<Vec<(String, adw::ExpanderRow)>>,
    run_rows: RefCell<Vec<(String, adw::ActionRow)>>,
    runs: RefCell<Vec<RunRecord>>,
    chosen: RefCell<Option<String>>,
    report: RefCell<Option<String>>,
    title: gtk::Label,
    facts: gtk::Label,
    details: gtk::TextView,
    message: gtk::Label,
    more: gtk::Button,
    copy: gtk::Button,
    export: gtk::Button,
    clear: gtk::Button,
    toasts: adw::ToastOverlay,
    loading: Cell<bool>,
    stale: Cell<bool>,
    exhausted: Cell<bool>,
    selecting: Cell<u64>,
    timer: RefCell<Option<glib::SourceId>>,
}

impl ConsolePage {
    pub fn new(app: &Rc<App>) -> Rc<Self> {
        let dates =
            gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).css_classes(["boxed-list"]).build();
        dates.update_property(&[gtk::accessible::Property::Label("Generation runs by date")]);
        let message = gtk::Label::builder()
            .label("No runs yet")
            .wrap(true)
            .selectable(true)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .margin_start(24)
            .margin_end(24)
            .css_classes(["secondary-text"])
            .build();
        let more = gtk::Button::builder().label("Load older runs").visible(false).build();
        let list = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        list.append(&dates);
        list.append(&more);
        let list_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_width(220)
            .vexpand(true)
            .child(&list)
            .build();
        let list_stack = gtk::Stack::builder().vexpand(true).hexpand(true).build();
        list_stack.add_named(&list_scroll, Some("runs"));
        list_stack.add_named(&message, Some("empty"));
        list_stack.set_visible_child_name("empty");

        let title = gtk::Label::builder()
            .label("Choose a run")
            .xalign(0.0)
            .wrap(true)
            .selectable(true)
            .css_classes(["title-2"])
            .build();
        let facts = gtk::Label::builder().xalign(0.0).wrap(true).selectable(true).build();
        let copy = gtk::Button::builder().label("Copy Details").sensitive(false).build();
        let export = gtk::Button::builder().label("Export JSON…").sensitive(false).build();
        let actions = adw::WrapBox::builder().child_spacing(8).line_spacing(8).build();
        actions.append(&copy);
        actions.append(&export);
        let heading = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        heading.append(&title);
        heading.append(&facts);
        heading.append(&actions);
        let details = gtk::TextView::builder()
            .editable(false)
            .cursor_visible(true)
            .monospace(true)
            .wrap_mode(gtk::WrapMode::WordChar)
            .left_margin(12)
            .right_margin(12)
            .top_margin(12)
            .bottom_margin(12)
            .build();
        details.update_property(&[gtk::accessible::Property::Label("Run requests, responses and outcomes")]);
        let detail_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&details)
            .build();
        let detail = gtk::Box::builder().orientation(gtk::Orientation::Vertical).vexpand(true).build();
        detail.append(&heading);
        detail.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        detail.append(&detail_scroll);
        let empty_detail = gtk::Label::builder()
            .label("Choose a run")
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .css_classes(["secondary-text"])
            .build();
        let detail_stack = gtk::Stack::builder().vexpand(true).hexpand(true).build();
        detail_stack.add_named(&detail, Some("run"));
        detail_stack.add_named(&empty_detail, Some("empty"));
        detail_stack.set_visible_child_name("empty");
        let panes = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .start_child(&list_stack)
            .end_child(&detail_stack)
            .position(280)
            .resize_start_child(false)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .vexpand(true)
            .build();
        let clear = gtk::Button::builder().icon_name("edit-clear-all-symbolic").tooltip_text("Clear console…").build();
        clear.update_property(&[gtk::accessible::Property::Label("Clear console")]);
        let refresh = gtk::Button::builder().icon_name("view-refresh-symbolic").tooltip_text("Refresh console").build();
        refresh.update_property(&[gtk::accessible::Property::Label("Refresh console")]);
        let header = adw::HeaderBar::new();
        header.pack_end(&clear);
        header.pack_end(&refresh);
        header.pack_end(&crate::window::progress_button(app));
        let totals = gtk::Label::builder().xalign(0.0).wrap(true).selectable(true).build();
        let outcomes = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build();
        let activity = crate::summary::ActivityChart::new();
        activity.update_property(&[gtk::accessible::Property::Label("Runs by day and outcome, UTC")]);
        let daily_table = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).build();
        daily_table.update_property(&[gtk::accessible::Property::Label("Daily run counts, UTC")]);
        let table = gtk::Expander::builder().label("Show counts").child(&daily_table).build();
        let daily = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build();
        let daily_legend = adw::WrapBox::builder().child_spacing(12).line_spacing(4).build();
        daily.append(&activity);
        daily.append(&daily_legend);
        daily.append(&table);
        let model_times = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).vexpand(true).build();
        let cards = adw::WrapBox::builder()
            .child_spacing(12)
            .line_spacing(12)
            .wrap_policy(adw::WrapPolicy::Minimum)
            .justify(adw::JustifyMode::Fill)
            .build();
        cards.append(&statistics_card("Outcomes", &outcomes));
        cards.append(&statistics_card("Runs by day · UTC", &daily));
        cards.append(&statistics_card("Average model call", &model_times));
        let charts = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).build();
        charts.append(&totals);
        charts.append(&cards);
        let chart_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .max_content_height(310)
            .propagate_natural_height(true)
            .child(&charts)
            .build();
        let statistics = gtk::Expander::builder()
            .label("Statistics")
            .expanded(true)
            .visible(false)
            .vexpand(false)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .child(&chart_scroll)
            .build();
        let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).vexpand(true).build();
        content.append(&statistics);
        content.append(&panes);
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&content));
        let root = adw::ToolbarView::new();
        root.add_top_bar(&header);
        root.set_content(Some(&toasts));

        let this = Rc::new(Self {
            root,
            app: Rc::downgrade(app),
            panes,
            list_stack,
            detail_stack,
            statistics,
            totals,
            outcomes,
            activity,
            daily_legend,
            daily_table,
            model_times,
            dates,
            date_rows: RefCell::new(Vec::new()),
            run_rows: RefCell::new(Vec::new()),
            runs: RefCell::new(Vec::new()),
            chosen: RefCell::new(None),
            report: RefCell::new(None),
            title,
            facts,
            details,
            message,
            more,
            copy,
            export,
            clear,
            toasts,
            loading: Cell::new(false),
            stale: Cell::new(true),
            exhausted: Cell::new(false),
            selecting: Cell::new(0),
            timer: RefCell::new(None),
        });
        let weak = Rc::downgrade(&this);
        refresh.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.load(false);
            }
        });
        this.connect();
        let weak = Rc::downgrade(&this);
        app.subscribe(move |event| {
            let Some(this) = weak.upgrade() else { return false };
            if matches!(event, Event::Ready | Event::History | Event::Progress) {
                this.update_actions();
                if this.root.is_mapped() {
                    this.load(false);
                } else {
                    this.stale.set(true);
                }
            }
            true
        });
        this.update_actions();
        this
    }

    pub fn widget(&self) -> &adw::ToolbarView {
        &self.root
    }

    pub fn set_stacked(&self, stacked: bool) {
        let orientation = if stacked { gtk::Orientation::Vertical } else { gtk::Orientation::Horizontal };
        if self.panes.orientation() != orientation {
            self.panes.set_resize_start_child(stacked);
            self.panes.set_orientation(orientation);
            self.panes.set_position(if stacked { 200 } else { 280 });
        }
    }

    fn connect(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.root.connect_map(move |_| {
            let Some(this) = weak.upgrade() else { return };
            if this.stale.get() {
                this.load(false);
            }
            let weak = Rc::downgrade(&this);
            let timer = glib::timeout_add_local(Duration::from_secs(2), move || {
                let Some(this) = weak.upgrade() else { return glib::ControlFlow::Break };
                if this.app.upgrade().is_some_and(|app| app.is_busy()) {
                    this.load(false);
                }
                glib::ControlFlow::Continue
            });
            this.timer.replace(Some(timer));
        });
        let weak = Rc::downgrade(self);
        self.root.connect_unmap(move |_| {
            if let Some(this) = weak.upgrade()
                && let Some(timer) = this.timer.take()
            {
                timer.remove();
            }
        });
        let weak = Rc::downgrade(self);
        self.more.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.load(true);
            }
        });
        let weak = Rc::downgrade(self);
        self.copy.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade()
                && let Some(report) = this.report.borrow().as_deref()
            {
                this.root.clipboard().set_text(report);
                this.toasts.add_toast(crate::window::toast("Run details copied"));
            }
        });
        let weak = Rc::downgrade(self);
        self.export.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.export();
            }
        });
        let weak = Rc::downgrade(self);
        self.clear.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.confirm_clear();
            }
        });
    }

    fn update_actions(&self) {
        let ready = self.app.upgrade().is_some_and(|app| app.is_ready() && !app.is_busy());
        self.clear.set_sensitive(ready && !self.runs.borrow().is_empty());
    }

    /// Reload what is already paged in, or request one older page. Keep selection and expanded dates across refreshes.
    fn load(self: &Rc<Self>, older: bool) {
        let Some(app) = self.app.upgrade() else { return };
        let Some(engine) = app.engine() else { return };
        if self.loading.replace(true) {
            self.stale.set(true);
            return;
        }
        self.stale.set(false);
        self.more.set_sensitive(false);
        let limit = (self.runs.borrow().len() as u32).max(PAGE_SIZE) + if older { PAGE_SIZE } else { 0 };
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = blocking(move || {
                let mut runs = Vec::new();
                let mut exhausted = false;
                while runs.len() < limit as usize {
                    let page = engine.runs(PAGE_SIZE.min(limit - runs.len() as u32), runs.len() as u32)?;
                    exhausted = page.len() < PAGE_SIZE as usize;
                    runs.extend(page);
                    if exhausted {
                        break;
                    }
                }
                let statistics = engine.console_statistics()?;
                Ok((runs, exhausted, statistics))
            })
            .await;
            let Some(this) = weak.upgrade() else { return };
            this.loading.set(false);
            this.more.set_sensitive(true);
            match result {
                Ok((runs, exhausted, statistics)) => {
                    this.exhausted.set(exhausted);
                    let same = snapshot(&runs) == snapshot(&this.runs.borrow());
                    this.runs.replace(runs);
                    if !same {
                        this.show_runs();
                    }
                    this.more.set_visible(!this.exhausted.get());
                    this.list_stack.set_visible_child_name(if this.runs.borrow().is_empty() {
                        "empty"
                    } else {
                        "runs"
                    });
                    this.message.set_label("No runs yet");
                    this.show_statistics(&statistics);
                    this.update_actions();
                    let chosen = this.chosen.borrow().clone();
                    if !same || this.report.borrow().is_none() || this.app.upgrade().is_some_and(|app| app.is_busy()) {
                        let id = chosen
                            .filter(|id| this.runs.borrow().iter().any(|run| run.id == *id))
                            .or_else(|| this.runs.borrow().first().map(|run| run.id.clone()));
                        match id {
                            Some(id) => this.select(id),
                            None => this.clear_selection(),
                        }
                    }
                }
                Err(error) => {
                    this.message.set_label(&format!("Couldn't load console: {error}"));
                    this.list_stack.set_visible_child_name("empty");
                }
            }
            if this.stale.get() && this.root.is_mapped() {
                this.load(false);
            }
        });
    }

    fn show_statistics(&self, statistics: &ConsoleStatistics) {
        self.statistics.set_visible(statistics.total > 0);
        if statistics.total == 0 {
            return;
        }
        let mut totals =
            format!("{} recorded {}", statistics.total, if statistics.total == 1 { "run" } else { "runs" });
        if let Some(rate) = statistics.success_rate {
            totals.push_str(&format!(" · {:.1}% success rate", rate * 100.0));
        }
        if let Some(seconds) = statistics.average_run_secs {
            totals.push_str(&format!(" · {} average run", duration(seconds)));
        }
        self.totals.set_label(&totals);
        self.totals.set_tooltip_text(Some(
            "Success rate is completed ÷ (completed + failed). Average run duration uses those same outcomes.",
        ));
        self.totals.update_property(&[gtk::accessible::Property::Description(
            "Success rate and average run duration use completed and failed runs.",
        )]);
        clear_box(&self.outcomes);
        let largest = statistics.outcomes.iter().map(|value| value.count).max().unwrap_or(1).max(1);
        for status in OUTCOMES {
            let count = statistics.outcomes.iter().find(|value| value.status == status).map_or(0, |value| value.count);
            if count > 0 {
                self.outcomes.append(&bar_row(outcome(status).0, &count.to_string(), count as f64, largest as f64));
            }
        }
        let data = daily_data(statistics);
        while let Some(child) = self.daily_legend.first_child() {
            self.daily_legend.remove(&child);
        }
        for (slot, status) in OUTCOMES.into_iter().enumerate() {
            if statistics.outcomes.iter().any(|value| value.status == status && value.count > 0) {
                let key = gtk::Box::builder().spacing(6).build();
                let dot = gtk::Box::builder()
                    .width_request(10)
                    .height_request(10)
                    .valign(gtk::Align::Center)
                    .css_classes(["chart-dot", &format!("slot-{slot}")])
                    .accessible_role(gtk::AccessibleRole::Presentation)
                    .build();
                key.append(&dot);
                key.append(&gtk::Label::new(Some(outcome(status).0)));
                self.daily_legend.append(&key);
            }
        }
        clear_box(&self.daily_table);
        for (start, stack) in data.starts.iter().zip(&data.stacks) {
            let day = glib::DateTime::from_unix_utc(*start)
                .and_then(|date| date.format("%-e %b"))
                .map(|value| value.to_string())
                .unwrap_or_default();
            let values = stack.iter().map(|segment| format!("{} {}", segment.name, segment.count)).collect::<Vec<_>>();
            if !values.is_empty() {
                let row = gtk::Label::builder()
                    .label(format!("{day}: {}", values.join(", ")))
                    .xalign(0.0)
                    .wrap(true)
                    .selectable(true)
                    .build();
                self.daily_table.append(&row);
            }
        }
        let daily_text =
            statistics.days.iter().map(|day| format!("{} {}", outcome(day.status).0, day.count)).collect::<Vec<_>>();
        self.activity.update_property(&[gtk::accessible::Property::Description(&format!(
            "{}. Show counts lists each day and outcome.",
            daily_text.join(", ")
        ))]);
        self.activity.set_data(data);
        clear_box(&self.model_times);
        if statistics.models.is_empty() {
            self.model_times.append(
                &gtk::Label::builder()
                    .label("No timed calls yet")
                    .halign(gtk::Align::Center)
                    .valign(gtk::Align::Center)
                    .vexpand(true)
                    .css_classes(["secondary-text"])
                    .build(),
            );
        } else {
            let longest = statistics.models.iter().map(|model| model.average_secs).fold(0.0_f64, f64::max);
            for model in &statistics.models {
                let job = match model.job {
                    ProviderJob::Concepts => "Writing",
                    ProviderJob::Images => "Painting",
                };
                let title = format!("{job} · {} · {}", strings::provider_name(model.provider), model.model);
                let value = format!(
                    "{} · {} {}",
                    duration(model.average_secs),
                    model.calls,
                    if model.calls == 1 { "call" } else { "calls" }
                );
                self.model_times.append(&bar_row(&title, &value, model.average_secs, longest.max(0.001)));
            }
        }
    }

    fn show_runs(self: &Rc<Self>) {
        let window = self.root.root().and_downcast::<gtk::Window>();
        let focus = window.as_ref().and_then(gtk::prelude::RootExt::focus);
        let focused = self
            .run_rows
            .borrow()
            .iter()
            .find(|(_, row)| {
                focus.as_ref().is_some_and(|focus| focus == row.upcast_ref::<gtk::Widget>() || focus.is_ancestor(row))
            })
            .map(|(id, _)| id.clone());
        let old: Vec<(String, bool)> =
            self.date_rows.borrow().iter().map(|(date, row)| (date.clone(), row.is_expanded())).collect();
        while let Some(child) = self.dates.first_child() {
            self.dates.remove(&child);
        }
        self.date_rows.borrow_mut().clear();
        self.run_rows.borrow_mut().clear();
        let mut previous = String::new();
        let mut section: Option<adw::ExpanderRow> = None;
        for run in self.runs.borrow().iter() {
            let date = date_key(run.started_at);
            if date != previous {
                let expanded = old
                    .iter()
                    .find(|(key, _)| *key == date)
                    .map(|(_, expanded)| *expanded)
                    .unwrap_or(self.date_rows.borrow().is_empty());
                let row = adw::ExpanderRow::builder()
                    .title(strings::iso_date(&date))
                    .use_markup(false)
                    .expanded(expanded)
                    .build();
                self.dates.append(&row);
                self.date_rows.borrow_mut().push((date.clone(), row.clone()));
                previous = date;
                section = Some(row);
            }
            let (outcome, icon) = outcome(run.status);
            let row = adw::ActionRow::builder()
                .title(format!("{} · {} · {outcome}", run_time(run.started_at), run.mood_name))
                .subtitle(if run.detail.is_empty() { "Generation started" } else { &run.detail })
                .subtitle_lines(2)
                .use_markup(false)
                .activatable(true)
                .build();
            row.add_prefix(&crate::history::decorative_icon(icon));
            row.add_suffix(&crate::history::decorative_icon("go-next-symbolic"));
            self.run_rows.borrow_mut().push((run.id.clone(), row.clone()));
            let weak = Rc::downgrade(self);
            let id = run.id.clone();
            row.connect_activated(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.select(id.clone());
                }
            });
            if let Some(section) = &section {
                section.add_row(&row);
            }
        }
        self.show_selection();
        if let Some(id) = focused
            && let Some((_, row)) = self.run_rows.borrow().iter().find(|(run, _)| *run == id)
        {
            row.grab_focus();
        }
    }

    fn show_selection(&self) {
        for (id, row) in self.run_rows.borrow().iter() {
            let selected = self.chosen.borrow().as_deref() == Some(id);
            if selected {
                row.set_state_flags(gtk::StateFlags::SELECTED, false);
            } else {
                row.unset_state_flags(gtk::StateFlags::SELECTED);
            }
            row.update_state(&[gtk::accessible::State::Selected(Some(selected))]);
        }
    }

    fn select(self: &Rc<Self>, id: String) {
        let Some(engine) = self.app.upgrade().and_then(|app| app.engine()) else { return };
        let changing = self.chosen.borrow().as_deref() != Some(&id);
        self.chosen.replace(Some(id.clone()));
        self.show_selection();
        if changing {
            self.report.replace(None);
            self.copy.set_sensitive(false);
            self.export.set_sensitive(false);
        }
        let epoch = self.selecting.get() + 1;
        self.selecting.set(epoch);
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = blocking(move || Ok((engine.run(id.clone())?, engine.run_report(id)?))).await;
            let Some(this) = weak.upgrade() else { return };
            if this.selecting.get() != epoch {
                return;
            }
            match result {
                Ok((run, report)) => {
                    this.detail_stack.set_visible_child_name("run");
                    this.title.set_label(&format!("{} · {}", run.mood_name, outcome(run.status).0));
                    this.facts.set_label(&format!(
                        "{}\nWriting: {} · {}\nPainting: {} · {}",
                        strings::day_label(run.started_at),
                        strings::provider_name(run.text_provider),
                        run.text_model,
                        strings::provider_name(run.image_provider),
                        run.image_model
                    ));
                    let text = run_details(&run);
                    let buffer = this.details.buffer();
                    let (start, end) = buffer.bounds();
                    if buffer.text(&start, &end, true).as_str() != text {
                        buffer.set_text(&text);
                    }
                    this.report.replace(Some(report));
                    this.copy.set_sensitive(true);
                    this.export.set_sensitive(true);
                }
                Err(error) => this.details.buffer().set_text(&format!("Couldn't load this run: {error}")),
            }
        });
    }

    fn clear_selection(&self) {
        self.selecting.set(self.selecting.get() + 1);
        self.chosen.replace(None);
        self.show_selection();
        self.report.replace(None);
        self.title.set_label("Choose a run");
        self.facts.set_label("");
        self.details.buffer().set_text("");
        self.detail_stack.set_visible_child_name("empty");
        self.copy.set_sensitive(false);
        self.export.set_sensitive(false);
    }

    fn export(self: &Rc<Self>) {
        let Some(report) = self.report.borrow().clone() else { return };
        let Some(id) = self.chosen.borrow().clone() else { return };
        let parent = self.root.root().and_downcast::<gtk::Window>();
        let dialog = gtk::FileDialog::builder()
            .title("Export run details")
            .initial_name(format!("AutoPaper-run-{id}.json"))
            .build();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let file = match dialog.save_future(parent.as_ref()).await {
                Ok(file) => file,
                Err(error)
                    if error.matches(gtk::DialogError::Dismissed) || error.matches(gtk::DialogError::Cancelled) =>
                {
                    return;
                }
                Err(error) => {
                    if let Some(this) = weak.upgrade() {
                        this.toasts.add_toast(crate::window::toast(&format!("Couldn't choose a file: {error}")));
                    }
                    return;
                }
            };
            let result =
                file.replace_contents_future(report.into_bytes(), None, false, gio::FileCreateFlags::PRIVATE).await;
            if let Some(this) = weak.upgrade() {
                let text = match result {
                    Ok(_) => "Run exported".to_string(),
                    Err((_, error)) => format!("Couldn't export this run: {error}"),
                };
                this.toasts.add_toast(crate::window::toast(&text));
            }
        });
    }

    fn confirm_clear(self: &Rc<Self>) {
        let Some(app) = self.app.upgrade() else { return };
        if app.is_busy() {
            return;
        }
        let alert = adw::AlertDialog::new(
            Some("Clear console?"),
            Some(
                "Deletes the local run requests, responses and outcomes. Your wallpapers, idea memory and budget spending are kept.",
            ),
        );
        alert.add_responses(&[("cancel", "_Cancel"), ("clear", "_Clear console")]);
        alert.set_response_appearance("clear", adw::ResponseAppearance::Destructive);
        alert.set_default_response(Some("cancel"));
        alert.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        let anchor = self.root.clone();
        glib::spawn_future_local(async move {
            if alert.choose_future(Some(&anchor)).await != "clear" {
                return;
            }
            let result = app.serial(|engine| engine.clear_runs()).await;
            if let Some(this) = weak.upgrade() {
                match result {
                    Ok(()) => {
                        this.clear_selection();
                        this.load(false);
                        this.toasts.add_toast(crate::window::toast("Console cleared"));
                    }
                    Err(error) => this.toasts.add_toast(crate::window::toast(&strings::error_sentence(
                        &error,
                        app.state().settings.fallback,
                    ))),
                }
            }
        });
    }
}

fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn statistics_card(title: &str, content: &impl IsA<gtk::Widget>) -> gtk::Box {
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .hexpand(true)
        .build();
    body.append(&gtk::Label::builder().label(title).xalign(0.0).css_classes(["heading"]).build());
    body.append(content);
    let card = gtk::Box::builder().width_request(200).hexpand(true).css_classes(["card"]).build();
    card.append(&body);
    card
}

fn bar_row(title: &str, value: &str, amount: f64, maximum: f64) -> gtk::Box {
    let row = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).build();
    let label =
        gtk::Label::builder().label(format!("{title}: {value}")).xalign(0.0).wrap(true).selectable(true).build();
    let bar = gtk::LevelBar::builder().min_value(0.0).max_value(maximum).value(amount).build();
    // These compare measured quantities; a full bar doesn't mean a healthy budget or a successful call.
    for offset in ["low", "high", "full"] {
        bar.remove_offset_value(Some(offset));
    }
    bar.update_property(&[gtk::accessible::Property::Label(&format!("{title}: {value}"))]);
    row.append(&label);
    row.append(&bar);
    row
}

fn duration(seconds: f64) -> String {
    if seconds < 0.1 { format!("{seconds:.3} s") } else { format!("{seconds:.1} s") }
}

/// Preserve every retained UTC day, including zero-run gaps; the shared chart owns drawing and tooltip text.
fn daily_data(statistics: &ConsoleStatistics) -> crate::summary::ChartData {
    let mut starts: Vec<_> = statistics.days.iter().map(|day| day.day_start).collect();
    starts.sort_unstable();
    starts.dedup();
    if let (Some(first), Some(last)) = (starts.first().copied(), starts.last().copied()) {
        starts = (first..=last).step_by(86_400).collect();
    }
    let stacks = starts
        .iter()
        .map(|start| {
            OUTCOMES
                .into_iter()
                .enumerate()
                .filter_map(|(slot, status)| {
                    statistics.days.iter().find(|day| day.day_start == *start && day.status == status).map(|day| {
                        crate::summary::Segment {
                            slot: Some(slot),
                            name: outcome(status).0.into(),
                            count: day.count as u32,
                        }
                    })
                })
                .collect()
        })
        .collect();
    crate::summary::ChartData { stacks, starts, utc: true }
}

fn date_key(unix: i64) -> String {
    glib::DateTime::from_unix_local(unix)
        .and_then(|time| time.format("%Y-%m-%d"))
        .map(|text| text.to_string())
        .unwrap_or_else(|_| "Unknown date".into())
}

fn run_time(unix: i64) -> String {
    glib::DateTime::from_unix_local(unix)
        .and_then(|time| time.format("%H:%M:%S"))
        .map(|text| text.to_string())
        .unwrap_or_default()
}

fn outcome(status: RunStatus) -> (&'static str, &'static str) {
    match status {
        RunStatus::Running => ("Running", "content-loading-symbolic"),
        RunStatus::Succeeded => ("Completed", "emblem-ok-symbolic"),
        RunStatus::Failed => ("Failed", "dialog-error-symbolic"),
        RunStatus::Blocked => ("Blocked", "dialog-warning-symbolic"),
        RunStatus::Cancelled => ("Cancelled", "process-stop-symbolic"),
        RunStatus::Interrupted => ("Interrupted", "dialog-warning-symbolic"),
    }
}

fn snapshot(runs: &[RunRecord]) -> Vec<(String, &'static str, usize, Option<i64>, String)> {
    runs.iter()
        .map(|run| (run.id.clone(), outcome(run.status).0, run.events.len(), run.finished_at, run.detail.clone()))
        .collect()
}

fn run_details(run: &RunRecord) -> String {
    let mut text = format!(
        "Run: {}\nOutcome: {}\nTrigger: {:?}\nStarted: {} {}\nMood: {}\nSurprise: {:.0}%\nWriting: {} / {}\nPainting: {} / {}\nCost estimate: {}\n{}\n",
        run.id,
        outcome(run.status).0,
        run.trigger,
        date_key(run.started_at),
        run_time(run.started_at),
        run.mood_name,
        run.surprise * 100.0,
        strings::provider_name(run.text_provider),
        run.text_model,
        strings::provider_name(run.image_provider),
        run.image_model,
        strings::console_money(run.cost_microusd),
        run.detail
    );
    if let Some(finished) = run.finished_at {
        text.push_str(&format!(
            "Finished: {} {}\nDuration: {} seconds\n",
            date_key(finished),
            run_time(finished),
            finished.saturating_sub(run.started_at)
        ));
    }
    for line in strings::keyword_groups(run.keywords.iter().map(|keyword| (keyword.text.as_str(), keyword.weight))) {
        text.push_str(&format!("{line}\n"));
    }
    if let Some(id) = &run.generation_id {
        text.push_str(&format!("Wallpaper: {id}\n"));
    }
    if run.events.is_empty() {
        text.push_str("\nNo provider request was recorded for this attempt.\n");
    }
    for event in &run.events {
        let provider = event.provider.map(strings::provider_name).unwrap_or("AutoPaper");
        let detail = serde_json::from_str::<serde_json::Value>(&event.detail)
            .ok()
            .and_then(|value| serde_json::to_string_pretty(&value).ok())
            .unwrap_or_else(|| event.detail.clone());
        text.push_str(&format!(
            "\n── {} · {} · {} ──\n{} / {}\n{}\n",
            run_time(event.at),
            event.stage,
            event.kind,
            provider,
            event.model,
            detail
        ));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use autopaper_core::ConsoleDay;

    fn statistics(days: Vec<ConsoleDay>) -> ConsoleStatistics {
        ConsoleStatistics {
            total: days.iter().map(|day| day.count).sum(),
            outcomes: Vec::new(),
            days,
            models: Vec::new(),
            average_run_secs: None,
            success_rate: None,
        }
    }

    #[test]
    fn daily_chart_keeps_utc_gaps_and_distinct_outcomes() {
        let data = daily_data(&statistics(vec![
            ConsoleDay { day_start: 172_800, status: RunStatus::Blocked, count: 3 },
            ConsoleDay { day_start: 0, status: RunStatus::Failed, count: 1 },
            ConsoleDay { day_start: 0, status: RunStatus::Succeeded, count: 2 },
        ]));
        assert!(data.utc);
        assert_eq!(data.starts, [0, 86_400, 172_800]);
        assert_eq!(
            data.stacks[0].iter().map(|value| (value.name.as_str(), value.count)).collect::<Vec<_>>(),
            [("Completed", 2), ("Failed", 1)]
        );
        assert!(data.stacks[1].is_empty());
        assert_eq!(data.stacks[2][0].name, "Blocked");
        assert_eq!(data.stacks[2][0].slot, Some(2));
    }

    #[test]
    fn empty_chart_has_no_invented_days_or_counts() {
        let data = daily_data(&statistics(Vec::new()));
        assert!(data.starts.is_empty());
        assert!(data.stacks.is_empty());
    }

    #[test]
    fn duration_preserves_fast_actual_calls() {
        assert_eq!(duration(0.004), "0.004 s");
        assert_eq!(duration(3.25), "3.2 s");
        assert_eq!(duration(120.0), "120.0 s");
    }
}
