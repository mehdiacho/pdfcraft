//! The compact layout for phones and narrow windows (the Android app's design, `Main`, `Tools`,
//! `Settings`, `Viewer` and `Contents` boards).
//!
//! Below [`COMPACT_WIDTH`] points wide, or [`COMPACT_HEIGHT`] tall (a phone held sideways), the
//! desktop chrome doesn't fit: the tab strip, mode bar, rail
//! and docked side panels give way to one app bar at the top and a bottom navigation bar, and
//! the side panels become full-width pages and bottom sheets. The switch is a pure function of
//! the window's size ([`is_compact`]), so a narrow desktop window gets the same layout and tests
//! can drive it. The desktop state (mode, docking, open panels) is left alone, so widening the
//! window brings the desktop layout back as it was.

use egui::{Color32, CornerRadius, Rect, Sense, Stroke, Vec2, vec2};
use pdfcraft_engine::catalog::{self, TOOL_GROUPS, ToolGroup};

use crate::theme::{self, ThemePreference, Tokens};
use crate::{Dialog, LeftPanel, PdfCraftApp, QuickTool, RightPanel, icons};

/// Windows narrower than this (in points) use the compact layout: every phone held upright, but
/// no tablet or desktop window at its usual size.
pub const COMPACT_WIDTH: f32 = 600.0;
/// Windows shorter than this also do: a phone held sideways is 800–950 points wide but only
/// 360–430 tall, too short for the desktop's stacked bars. Tablets held sideways are 600 or more.
pub const COMPACT_HEIGHT: f32 = 480.0;

/// The design's touch target and bar heights.
const TOUCH: f32 = 48.0;
const APP_BAR: f32 = 64.0;
const NAV_BAR: f32 = 72.0;
/// Side margin of the compact pages.
const GUTTER: f32 = 16.0;

/// Whether a window of `size` points uses the compact layout. An unknown (non-finite) side
/// doesn't count.
pub fn is_compact(size: Vec2) -> bool {
    (size.x.is_finite() && size.x < COMPACT_WIDTH) || (size.y.is_finite() && size.y < COMPACT_HEIGHT)
}

/// Whether this frame lays out compactly (for widgets deep in the tree, such as menus that would
/// otherwise show keyboard shortcuts).
pub(crate) fn active(ctx: &egui::Context) -> bool {
    is_compact(ctx.content_rect().size())
}

/// The page the bottom navigation bar shows. With a document open, [`Page::Home`] is the document.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Home,
    Tools,
    Settings,
}

/// What a sheet over the bottom of the screen shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sheet {
    /// The side panels (pages, bookmarks, comments …), with a tab for each.
    Contents,
    /// The chosen tool's panel. Without a document it fills the screen.
    Tool,
}

/// A family of tools, as the All tools page's filter chips group them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Category {
    #[default]
    All,
    Edit,
    Convert,
    Sign,
    Advanced,
}

impl Category {
    const ORDER: [Self; 5] = [Self::All, Self::Edit, Self::Convert, Self::Sign, Self::Advanced];

    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Edit => "Edit",
            Self::Convert => "Convert",
            Self::Sign => "Sign",
            Self::Advanced => "Advanced",
        }
    }

    /// The family a catalogue group belongs to; anything new lands under Advanced.
    pub fn of(group: &str) -> Self {
        match group {
            "edit" | "organize" | "combine" | "comment" | "stamp" | "redact" | "compress" | "measure" => Self::Edit,
            "export" | "create" | "scan" => Self::Convert,
            "fill_sign" | "protect" | "certificate" | "form" => Self::Sign,
            _ => Self::Advanced,
        }
    }
}

/// The compact layout's own state; nothing here is saved.
#[derive(Debug, Default)]
pub struct Compact {
    pub page: Page,
    pub sheet: Option<Sheet>,
    pub category: Category,
    /// The document shown last frame: opening or switching documents starts on the page itself.
    last_doc: Option<pdfcraft_engine::DocId>,
    /// Documents already fitted to the width (see [`sync`]).
    fitted: Vec<pdfcraft_engine::DocId>,
}

/// The Home grid: the tools the design puts first, two rows of four.
const HOME_TOOLS: [&str; 8] = ["edit", "fill_sign", "organize", "combine", "comment", "scan", "compress", "protect"];

/// The whole window in the compact layout.
pub fn show(app: &mut PdfCraftApp, ui: &mut egui::Ui) {
    sync(app);
    let t = Tokens::get(ui.ctx());
    // A drag that starts on text scrolls its page instead of selecting the text, for the mouse
    // too (egui does this itself once it has seen a touch).
    ui.style_mut().interaction.selectable_labels = false;
    app_bar(app, ui, &t);
    nav_bar(app, ui, &t);
    let has_doc = app.active.is_some();
    // With a document, sheets rise over the bottom of the page so the page stays in sight above.
    if has_doc && app.compact.page == Page::Home && app.compact.sheet.is_some() {
        let height = (ui.available_height() * if app.compact.sheet == Some(Sheet::Contents) { 0.62 } else { 0.5 }).max(TOUCH * 4.0);
        egui::Panel::bottom("compact_sheet")
            .exact_size(height)
            .resizable(false)
            .frame(egui::Frame::NONE.fill(t.panel).stroke(Stroke::new(1.0, t.divider)))
            .show(ui, |ui| sheet(app, ui, &t));
    }
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.pasteboard)).show(ui, |ui| match (app.active, app.compact.page) {
        (_, Page::Tools) => tools_page(app, ui, &t),
        (None, Page::Settings) => settings_page(app, ui, &t),
        (None, _) if app.combine_showing() => crate::combine_ui::page(app, ui),
        (None, _) if app.compact.sheet == Some(Sheet::Tool) => crate::panels::left_panel(app, ui),
        (None, _) => home(app, ui, &t),
        (Some(index), _) => match crate::compare_ui::side_by_side(app, index) {
            Some((old, new)) => crate::compare_ui::compare_area(app, old, new, ui),
            None => crate::canvas::document_area(app, index, ui),
        },
    });
}

/// Keeps the compact state in step with what panels and commands did since last frame.
fn sync(app: &mut PdfCraftApp) {
    let doc = app.active_ids().map(|(_, id)| id);
    if doc != app.compact.last_doc {
        // A document opened, or Home came back: show it, with nothing over it.
        app.compact.last_doc = doc;
        app.compact.page = Page::Home;
        app.compact.sheet = None;
        // A phone reads a page across its width, as the design's Viewer does; once per document,
        // so coming back to it keeps the zoom the reader chose.
        if let Some(id) = doc
            && !app.compact.fitted.contains(&id)
            && let Some(view) = app.active.and_then(|i| app.views.get_mut(i))
        {
            view.set_fit(crate::canvas::Fit::Width);
            app.compact.fitted.push(id);
        }
    }
    match app.compact.sheet {
        // The panel's own Close closes the sheet.
        Some(Sheet::Contents) if app.right.is_none() => app.compact.sheet = None,
        Some(Sheet::Tool) if !app.left_open => app.compact.sheet = None,
        // "Back to all tools" in a tool's panel: the All tools page.
        Some(Sheet::Tool) if app.left == LeftPanel::AllTools => {
            app.compact.sheet = None;
            app.compact.page = Page::Tools;
        }
        _ => {}
    }
}

/// Opens a tool's panel: a sheet under the document, or the whole screen without one.
fn open_tool(app: &mut PdfCraftApp, id: &'static str) {
    app.select_tool(id);
    app.compact.page = Page::Home;
    app.compact.sheet = Some(Sheet::Tool);
}

/// Back to reading: no sheet, no tool in hand, no page grid.
fn read(app: &mut PdfCraftApp) {
    app.compact.page = Page::Home;
    app.compact.sheet = None;
    if matches!(app.quick_tool, QuickTool::EditText | QuickTool::AddText) {
        app.put_down_edit_text();
    } else {
        app.quick_tool = QuickTool::Select;
    }
    if let Some(view) = app.active.and_then(|i| app.views.get_mut(i)) {
        view.organize = false;
    }
}

/// A single line of text cut with "…" to `max_width`.
fn line(ui: &egui::Ui, text: &str, font: egui::FontId, color: Color32, max_width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping { max_width: max_width.max(1.0), max_rows: 1, break_anywhere: true, ..Default::default() };
    ui.fonts_mut(|f| f.layout_job(job))
}

/// A round icon button with the design's 48-point touch target.
fn icon_button(ui: &mut egui::Ui, icon: &str, label: &str) -> egui::Response {
    icons::button(ui, icon, TOUCH, false, label)
}

fn app_bar(app: &mut PdfCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    // The document's name and page count, when the app bar is the document's.
    let doc = app
        .active_ids()
        .filter(|_| app.compact.page == Page::Home)
        .and_then(|(_, id)| app.session.get(id))
        .map(|d| (d.display_name(), d.info.pages.len()));
    let fill = if doc.is_some() { t.chrome } else { t.pasteboard };
    let stroke = if doc.is_some() { Stroke::new(1.0, t.divider) } else { Stroke::NONE };
    egui::Panel::top("compact_app_bar")
        .exact_size(APP_BAR)
        .frame(egui::Frame::NONE.fill(fill).stroke(stroke).inner_margin(egui::Margin { left: 4, right: 4, top: 0, bottom: 0 }))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                match doc {
                    Some((name, pages)) => {
                        if icon_button(ui, "chevron-left", tl!("Home")).clicked() {
                            app.active = None;
                        }
                        // Name and page count, then find, contents and the overflow menu.
                        let width = (ui.available_width() - 3.0 * TOUCH - 4.0).max(0.0);
                        let (rect, _) = ui.allocate_exact_size(vec2(width, APP_BAR), Sense::hover());
                        let name = line(ui, &crate::bidi::visual(&name), theme::semibold(15.0), t.text, width - 4.0);
                        let count = crate::i18n::trn(crate::i18n::current(), pages as u64, "{n} page", "{n} pages");
                        let count = line(ui, &count, theme::regular(12.0), t.text_muted, width - 4.0);
                        let top = rect.center().y - (name.size().y + count.size().y + 2.0) / 2.0;
                        ui.painter().galley(egui::pos2(rect.left() + 4.0, top), name.clone(), t.text);
                        ui.painter().galley(egui::pos2(rect.left() + 4.0, top + name.size().y + 2.0), count, t.text_muted);
                        if icon_button(ui, "search", tl!("Find…")).clicked() {
                            read(app);
                            app.execute("edit.find");
                        }
                        let contents = app.compact.sheet == Some(Sheet::Contents);
                        if icons::button(ui, "list", TOUCH, contents, tl!("Side panels")).clicked() {
                            toggle_contents(app);
                        }
                        let more = icon_button(ui, "ellipsis-vertical", tl!("More options"));
                        egui::Popup::menu(&more).show(|ui| overflow_menu(app, ui));
                    }
                    None => {
                        ui.add_space(16.0);
                        let title = match app.compact.page {
                            Page::Tools => Some(tl!("All tools")),
                            Page::Settings => Some(tl!("Settings")),
                            Page::Home if app.active.is_none() => None,
                            Page::Home => Some(tl!("All tools")),
                        };
                        let width = (ui.available_width() - TOUCH - 4.0).max(0.0);
                        let (rect, _) = ui.allocate_exact_size(vec2(width, APP_BAR), Sense::hover());
                        let left = match title {
                            Some(_) => rect.left(),
                            None => {
                                // The app's mark: a page on the PDF red.
                                let mark = Rect::from_min_size(egui::pos2(rect.left(), rect.center().y - 15.0), vec2(30.0, 30.0));
                                ui.painter().rect_filled(mark, CornerRadius::same(8), Color32::from_rgb(0xE0, 0x3E, 0x3E));
                                icons::paint(ui, mark, "file-text", 18.0, Color32::WHITE);
                                mark.right() + 10.0
                            }
                        };
                        let title = line(ui, title.unwrap_or("PdfCraft"), theme::semibold(20.0), t.text, rect.right() - left);
                        ui.painter().galley(egui::pos2(left, rect.center().y - title.size().y / 2.0), title, t.text);
                        if icon_button(ui, "search", tl!("Find tools and commands")).clicked() {
                            app.palette_open = true;
                        }
                    }
                }
            });
        });
}

/// Opens the side panels' sheet on the panel last shown (or one the document has), or closes it.
fn toggle_contents(app: &mut PdfCraftApp) {
    if app.compact.sheet == Some(Sheet::Contents) {
        app.compact.sheet = None;
        return;
    }
    if app.right.is_none() {
        let outline = app.active_ids().and_then(|(_, id)| app.session.get(id)).is_some_and(|d| !d.info.outline.is_empty());
        app.choose_right_panel(Some(if outline { RightPanel::Bookmarks } else { RightPanel::Pages }));
    }
    app.compact.page = Page::Home;
    app.compact.sheet = Some(Sheet::Contents);
}

/// The overflow menu: the desktop toolbar's buttons and menus, without keyboard shortcuts.
fn overflow_menu(app: &mut PdfCraftApp, ui: &mut egui::Ui) {
    ui.set_min_width(240.0);
    ui.style_mut().spacing.button_padding = vec2(10.0, 10.0);
    for id in ["file.save", "file.save_as", "print.dialog", "file.properties"] {
        command_button(app, ui, id);
    }
    if ui.button(tl!("Export a PDF")).clicked() {
        open_tool(app, "export");
        ui.close();
    }
    command_button(app, ui, "file.close");
    ui.separator();
    for menu in ["File", "Edit", "Pages", "View", "Help"] {
        ui.menu_button(tl!(menu), |ui| crate::commands::registry_menu(app, ui, menu));
    }
    ui.menu_button(tl!("Display theme"), |ui| crate::chrome::theme_menu(app, ui));
}

/// A registered command as a menu entry, enabled as its menu would be.
fn command_button(app: &mut PdfCraftApp, ui: &mut egui::Ui, id: &str) {
    let Some(spec) = pdfcraft_engine::commands::command(id) else { return };
    let label = pdfcraft_engine::commands::current_label(spec, &app.session, app.active_ids().map(|(_, id)| id));
    let label = crate::i18n::menu_label(spec.id, &label);
    if ui.add_enabled(app.command_enabled(spec), egui::Button::new(label)).clicked() {
        app.execute(spec.id);
        ui.close();
    }
}

/// One destination of the bottom bar.
struct Destination {
    icon: &'static str,
    label: &'static str,
    selected: bool,
}

fn nav_bar(app: &mut PdfCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let page = app.compact.page;
    let tool = |id| app.compact.sheet == Some(Sheet::Tool) && app.left == LeftPanel::Tool(id);
    let organizing = app.active.is_some() && crate::organize_panel::showing(app);
    let items: Vec<Destination> = if app.active.is_some() {
        let tools = page == Page::Tools;
        vec![
            Destination { icon: "book-open", label: "Read", selected: !tools && !organizing && app.compact.sheet != Some(Sheet::Tool) },
            Destination { icon: "message-square-text", label: "Comment", selected: !tools && tool("comment") },
            Destination { icon: "signature", label: "Fill & Sign", selected: !tools && tool("fill_sign") },
            Destination { icon: "files", label: "Pages", selected: !tools && organizing },
            Destination { icon: "layout-grid", label: "Tools", selected: tools },
        ]
    } else {
        let tool_page = app.compact.sheet == Some(Sheet::Tool);
        vec![
            Destination { icon: "house", label: "Home", selected: page == Page::Home && !tool_page },
            Destination { icon: "layout-grid", label: "Tools", selected: page == Page::Tools || tool_page },
            Destination { icon: "settings", label: "Settings", selected: page == Page::Settings },
        ]
    };
    let mut picked = None;
    egui::Panel::bottom("compact_nav_bar")
        .exact_size(NAV_BAR)
        .resizable(false)
        .frame(egui::Frame::NONE.fill(t.chrome).stroke(Stroke::new(1.0, t.divider)))
        .show(ui, |ui| {
            let full = ui.max_rect();
            let width = full.width() / items.len() as f32;
            for (i, item) in items.iter().enumerate() {
                let rect = Rect::from_min_size(egui::pos2(full.left() + i as f32 * width, full.top()), vec2(width, full.height()));
                let resp = ui.interact(rect, ui.id().with(("compact_nav", i)), Sense::click());
                let label = tl!(item.label);
                resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, item.selected, label));
                let color = if item.selected { t.accent_text } else { t.text_muted };
                let pill = Rect::from_center_size(egui::pos2(rect.center().x, rect.top() + 24.0), vec2(56.0_f32.min(width - 8.0), 30.0));
                let press = theme::Press::track(ui, &resp);
                let fill = if item.selected { t.accent_soft } else { press.fill(Color32::TRANSPARENT, t.hover, t.pressed) };
                if fill.a() > 0 {
                    ui.painter().rect_filled(pill, CornerRadius::same(15), fill);
                }
                icons::paint(ui, pill, item.icon, 22.0, color);
                let font = if item.selected { theme::semibold(12.0) } else { theme::medium(12.0) };
                let text = line(ui, label, font, color, width - 6.0);
                ui.painter().galley(egui::pos2(rect.center().x - text.size().x / 2.0, pill.bottom() + 4.0), text, color);
                if resp.clicked() {
                    picked = Some(item.label);
                }
            }
        });
    match picked {
        Some("Home") => {
            app.compact.page = Page::Home;
            app.compact.sheet = None;
        }
        Some("Settings") => {
            app.compact.page = Page::Settings;
            app.compact.sheet = None;
        }
        Some("Tools") => {
            app.compact.page = Page::Tools;
            if app.active.is_none() {
                app.compact.sheet = None;
            }
        }
        Some("Read") => read(app),
        Some("Comment") => open_tool(app, "comment"),
        Some("Fill & Sign") => open_tool(app, "fill_sign"),
        Some("Pages") => open_tool(app, "organize"),
        _ => {}
    }
}

/// The sheet over the bottom of the document.
fn sheet(app: &mut PdfCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    // The grab handle the design draws at the top of every sheet.
    let top = ui.max_rect().top();
    ui.painter().rect_filled(Rect::from_center_size(egui::pos2(ui.max_rect().center().x, top + 8.0), vec2(36.0, 4.0)), 2.0, t.border);
    ui.add_space(14.0);
    match app.compact.sheet {
        Some(Sheet::Contents) => {
            contents_tabs(app, ui, t);
            crate::panels::right_panel(app, ui);
        }
        Some(Sheet::Tool) => crate::panels::left_panel(app, ui),
        None => {}
    }
}

/// The side panels, one tab each, wrapped onto a second row when a language needs it.
fn contents_tabs(app: &mut PdfCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    egui::Frame::NONE.inner_margin(egui::Margin::symmetric(12, 0)).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for (panel, label) in [
                (RightPanel::Pages, "Pages"),
                (RightPanel::Bookmarks, "Bookmarks"),
                (RightPanel::Comments, "Comments"),
                (RightPanel::Fields, "Fields"),
                (RightPanel::Layers, "Layers"),
                (RightPanel::Attachments, "Attachments"),
                (RightPanel::Signatures, "Signatures"),
            ] {
                if chip(ui, t, tl!(label), app.right == Some(panel)).clicked() {
                    app.choose_right_panel(Some(panel));
                }
            }
        });
    });
    ui.add_space(6.0);
}

/// A filter chip: dark when selected, outlined otherwise.
fn chip(ui: &mut egui::Ui, t: &Tokens, label: &str, selected: bool) -> egui::Response {
    let font = theme::medium(14.0);
    let max = (ui.available_width()).max(TOUCH);
    let text = line(ui, label, font, if selected { t.chrome } else { t.text }, max - 32.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(text.size().x + 32.0, 36.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, label));
    let press = theme::Press::track(ui, &resp);
    let (fill, stroke) = if selected { (t.text, Stroke::NONE) } else { (press.fill(t.card, t.hover, t.pressed), Stroke::new(1.0, t.border)) };
    ui.painter().rect(rect, CornerRadius::same(18), fill, stroke, egui::StrokeKind::Inside);
    let color = if selected { t.chrome } else { t.text };
    ui.painter().galley(rect.center() - text.size() / 2.0, text, color);
    theme::hand(resp)
}

/// A vertical page with the compact gutters, scrolled by dragging it as well as by the wheel.
///
/// egui drags a scroll area only once it has seen a touch screen (`DragScroll::OnTouch`, which
/// needs an `Event::Touch`). A phone page is dragged whatever the pointer: a mouse in the Android
/// emulator, a stylus or trackpad reported as a pointer, or a narrow desktop window.
fn page<R>(ui: &mut egui::Ui, id: &str, content: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::ScrollArea::vertical()
        .id_salt(id)
        .auto_shrink([false, false])
        .scroll_source(egui::scroll_area::ScrollSource { drag: egui::scroll_area::DragScroll::Always, ..Default::default() })
        .show(ui, |ui| {
            egui::Frame::NONE
                .inner_margin(egui::Margin { left: GUTTER as i8, right: GUTTER as i8, top: 8, bottom: 16 })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    content(ui)
                })
                .inner
        })
        .inner
}

fn section_heading(ui: &mut egui::Ui, t: &Tokens, text: &str) {
    ui.label(egui::RichText::new(text).font(theme::semibold(15.0)).color(t.text));
}

/// The design's white card around a list.
fn card<R>(ui: &mut egui::Ui, t: &Tokens, content: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::NONE
        .fill(t.card)
        .stroke(Stroke::new(1.0, t.divider))
        .corner_radius(CornerRadius::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            content(ui)
        })
        .inner
}

fn home(app: &mut PdfCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let mut action = None;
    page(ui, "compact_home", |ui| {
        ui.spacing_mut().item_spacing.y = 10.0;
        // Open a file, and Create beside it.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let create_w = 136.0_f32.min(ui.available_width() * 0.4);
            let open_w = (ui.available_width() - create_w - 10.0).max(0.0);
            let (rect, resp) = ui.allocate_exact_size(vec2(open_w, 56.0), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("Open file")));
            let press = theme::Press::track(ui, &resp);
            ui.painter().rect_filled(rect, CornerRadius::same(12), t.accent.lerp_to_gamma(Color32::BLACK, 0.12 * press.down));
            let label = line(ui, tl!("Open file"), theme::semibold(16.0), Color32::WHITE, rect.width() - 52.0);
            let left = rect.center().x - (label.size().x + 32.0) / 2.0;
            icons::paint(ui, Rect::from_min_size(egui::pos2(left, rect.center().y - 11.0), vec2(22.0, 22.0)), "folder-open", 22.0, Color32::WHITE);
            ui.painter().galley(egui::pos2(left + 32.0, rect.center().y - label.size().y / 2.0), label, Color32::WHITE);
            if theme::hand(resp).clicked() {
                action = Some(HomeAction::Open);
            }
            let (rect, resp) = ui.allocate_exact_size(vec2(create_w, 56.0), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("Create")));
            let press = theme::Press::track(ui, &resp);
            ui.painter().rect(
                rect,
                CornerRadius::same(12),
                press.fill(t.card, t.hover, t.pressed),
                Stroke::new(1.0, t.border),
                egui::StrokeKind::Inside,
            );
            let label = line(ui, tl!("Create"), theme::semibold(15.0), t.text, rect.width() - 44.0);
            let left = rect.center().x - (label.size().x + 30.0) / 2.0;
            let red = Color32::from_rgb(0xE0, 0x3E, 0x3E);
            icons::paint(ui, Rect::from_min_size(egui::pos2(left, rect.center().y - 11.0), vec2(22.0, 22.0)), "file-plus", 22.0, red);
            ui.painter().galley(egui::pos2(left + 30.0, rect.center().y - label.size().y / 2.0), label, t.text);
            if theme::hand(resp).clicked() {
                action = Some(HomeAction::Tool("create"));
            }
        });
        ui.add_space(10.0);

        // Tools, with the way to all of them.
        ui.horizontal(|ui| {
            section_heading(ui, t, tl!("Tools"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let all = ui.add(
                    egui::Label::new(egui::RichText::new(tl!("All tools")).font(theme::medium(14.0)).color(t.accent_text))
                        .selectable(false)
                        .sense(Sense::click()),
                );
                if theme::hand(all).clicked() {
                    action = Some(HomeAction::AllTools);
                }
            });
        });
        let gap = 8.0;
        let columns = 4;
        let tile_w = ((ui.available_width() - gap * (columns - 1) as f32) / columns as f32).floor();
        for row in HOME_TOOLS.chunks(columns) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for id in row {
                    let Some(g) = catalog::group(id) else { continue };
                    if tool_tile(ui, t, g, tile_w).clicked() {
                        action = Some(HomeAction::Tool(g.id));
                    }
                }
            });
        }
        ui.add_space(10.0);

        section_heading(ui, t, tl!("Recent"));
        if app.recent.is_empty() {
            // Not the desktop's "Drop a PDF anywhere": a phone has nothing to drop from.
            ui.label(egui::RichText::new(tl!("No recent files")).color(t.text_muted));
        } else {
            card(ui, t, |ui| {
                let last = app.recent.len().saturating_sub(1);
                for (i, r) in app.recent.iter().enumerate() {
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 64.0), Sense::click());
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &r.name));
                    let press = theme::Press::track(ui, &resp);
                    press.wash(ui, rect.shrink(1.0), 11, false);
                    if i < last {
                        ui.painter().hline(rect.x_range().shrink(1.0), rect.bottom(), Stroke::new(1.0, t.divider));
                    }
                    let red = Color32::from_rgb(0xE0, 0x3E, 0x3E);
                    icons::paint(
                        ui,
                        Rect::from_center_size(egui::pos2(rect.left() + 27.0, rect.center().y), vec2(26.0, 26.0)),
                        "file-text",
                        26.0,
                        red,
                    );
                    let width = rect.width() - 54.0 - 12.0;
                    let name = line(ui, &crate::bidi::visual(&r.name), theme::medium(15.0), t.text, width);
                    let pages = crate::i18n::trn(crate::i18n::current(), r.pages as u64, "{n} page", "{n} pages");
                    let detail = line(ui, &format!("{pages}  ·  {}", crate::panels::human_size(r.size)), theme::regular(12.0), t.text_muted, width);
                    let top = rect.center().y - (name.size().y + detail.size().y + 2.0) / 2.0;
                    ui.painter().galley(egui::pos2(rect.left() + 54.0, top), name.clone(), t.text);
                    ui.painter().galley(egui::pos2(rect.left() + 54.0, top + name.size().y + 2.0), detail, t.text_muted);
                    // A long press offers to take the file off the list (the file itself stays).
                    resp.context_menu(|ui| {
                        if ui.button(tl!("Remove from Recent")).clicked() {
                            action = Some(HomeAction::Forget(r.path.clone()));
                            ui.close();
                        }
                    });
                    if theme::hand(resp).clicked() {
                        action = Some(HomeAction::Recent(r.path.clone()));
                    }
                }
            });
        }
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(tl!("PdfCraft works offline. No telemetry, no account, and no cloud processing unless you add a provider."))
                .font(theme::regular(12.0))
                .color(t.text_faint),
        );
    });
    match action {
        Some(HomeAction::Open) => app.open_dialog(),
        Some(HomeAction::AllTools) => app.compact.page = Page::Tools,
        Some(HomeAction::Tool(id)) => open_tool(app, id),
        Some(HomeAction::Recent(path)) => app.open_recent(&path),
        Some(HomeAction::Forget(path)) => app.recent.retain(|r| r.path != path),
        None => {}
    }
}

enum HomeAction {
    Open,
    AllTools,
    Tool(&'static str),
    Recent(String),
    Forget(String),
}

fn hue(g: &ToolGroup) -> Color32 {
    Color32::from_rgb(g.hue[0], g.hue[1], g.hue[2])
}

/// The tool family's icon on a soft wash of its colour.
fn tool_icon(ui: &egui::Ui, t: &Tokens, g: &ToolGroup, center: egui::Pos2) {
    let tile = Rect::from_center_size(center, vec2(36.0, 36.0));
    let wash = if t.dark() { 0.22 } else { 0.11 };
    ui.painter().rect_filled(tile, CornerRadius::same(9), t.card.lerp_to_gamma(hue(g), wash));
    icons::paint(ui, tile, g.icon, 22.0, hue(g));
}

/// A Home grid tile: the icon over the tool's name (two lines at most).
fn tool_tile(ui: &mut egui::Ui, t: &Tokens, g: &ToolGroup, width: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 92.0), Sense::click());
    let label = tl!(g.label);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let press = theme::Press::track(ui, &resp);
    ui.painter().rect(rect, CornerRadius::same(10), press.fill(t.card, t.hover, t.pressed), Stroke::new(1.0, t.divider), egui::StrokeKind::Inside);
    tool_icon(ui, t, g, egui::pos2(rect.center().x, rect.top() + 30.0));
    let mut job = egui::text::LayoutJob::simple(label.to_owned(), theme::medium(12.0), t.text, rect.width() - 8.0);
    job.wrap.max_rows = 2;
    job.halign = egui::Align::Center;
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    ui.painter().galley(egui::pos2(rect.center().x, rect.top() + 54.0), galley, t.text);
    theme::hand(resp)
}

fn tools_page(app: &mut PdfCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let mut picked = None;
    let mut search = false;
    page(ui, "compact_tools", |ui| {
        ui.spacing_mut().item_spacing.y = 12.0;
        // The search field: the command palette, which finds tools and commands alike.
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), TOUCH), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("Find tools and commands")));
        let press = theme::Press::track(ui, &resp);
        ui.painter().rect(
            rect,
            CornerRadius::same(24),
            press.fill(t.field, t.hover, t.pressed),
            Stroke::new(1.0, t.border),
            egui::StrokeKind::Inside,
        );
        icons::paint(ui, Rect::from_center_size(egui::pos2(rect.left() + 26.0, rect.center().y), vec2(20.0, 20.0)), "search", 20.0, t.text_muted);
        let hint = line(ui, tl!("Find tools and commands"), theme::regular(15.0), t.text_faint, rect.width() - 60.0);
        ui.painter().galley(egui::pos2(rect.left() + 48.0, rect.center().y - hint.size().y / 2.0), hint, t.text_faint);
        search = theme::hand(resp).clicked();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            for c in Category::ORDER {
                if chip(ui, t, tl!(c.label()), app.compact.category == c).clicked() {
                    app.compact.category = c;
                }
            }
        });
        for c in Category::ORDER.into_iter().skip(1) {
            if app.compact.category != Category::All && app.compact.category != c {
                continue;
            }
            let groups: Vec<&ToolGroup> = TOOL_GROUPS.iter().filter(|g| Category::of(g.id) == c).collect();
            if groups.is_empty() {
                continue;
            }
            ui.add_space(4.0);
            section_heading(ui, t, tl!(c.label()));
            card(ui, t, |ui| {
                let last = groups.len().saturating_sub(1);
                for (i, g) in groups.iter().enumerate() {
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 56.0), Sense::click());
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!(g.label)));
                    let press = theme::Press::track(ui, &resp);
                    press.wash(ui, rect.shrink(1.0), 11, false);
                    if i < last {
                        ui.painter().hline(rect.x_range().shrink(1.0), rect.bottom(), Stroke::new(1.0, t.divider));
                    }
                    tool_icon(ui, t, g, egui::pos2(rect.left() + 32.0, rect.center().y));
                    let label = line(ui, tl!(g.label), theme::medium(15.0), t.text, rect.width() - 64.0 - 36.0);
                    ui.painter().galley(egui::pos2(rect.left() + 64.0, rect.center().y - label.size().y / 2.0), label, t.text);
                    icons::paint(
                        ui,
                        Rect::from_center_size(rect.right_center() - vec2(22.0, 0.0), vec2(18.0, 18.0)),
                        "chevron-right",
                        18.0,
                        t.text_faint,
                    );
                    if theme::hand(resp).clicked() {
                        picked = Some(g.id);
                    }
                }
            });
        }
    });
    if search {
        app.palette_open = true;
    }
    if let Some(id) = picked {
        open_tool(app, id);
    }
}

fn settings_page(app: &mut PdfCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let mut command = None;
    let mut dialog = None;
    page(ui, "compact_settings", |ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.label(egui::RichText::new(tl!("Appearance")).font(theme::semibold(13.0)).color(t.text_muted));
        card(ui, t, |ui| {
            for (preference, id, label) in [
                (ThemePreference::System, "view.theme.system", "Use system setting"),
                (ThemePreference::Light, "view.theme.light", "Light gray"),
                (ThemePreference::Dark, "view.theme.dark", "Dark gray"),
            ] {
                let selected = app.theme_preference == preference;
                let resp = settings_row(ui, t, tl!(label), if selected { Some("check") } else { None }, true);
                if resp.clicked() {
                    command = Some(id);
                }
            }
            if settings_row(ui, t, tl!("Appearance settings…"), Some("chevron-right"), false).clicked() {
                dialog = Some(Dialog::Preferences);
            }
        });
        ui.add_space(8.0);
        ui.label(egui::RichText::new(tl!("About")).font(theme::semibold(13.0)).color(t.text_muted));
        card(ui, t, |ui| {
            if settings_row(ui, t, tl!("About PdfCraft"), Some("chevron-right"), false).clicked() {
                dialog = Some(Dialog::About);
            }
        });
    });
    if let Some(id) = command {
        app.execute(id);
    }
    if dialog.is_some() {
        app.dialog = dialog;
    }
}

/// A 56-point settings row: its label, and a trailing icon (a check, or a chevron to go on).
fn settings_row(ui: &mut egui::Ui, t: &Tokens, label: &str, trailing: Option<&str>, divider: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 56.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, trailing == Some("check"), label));
    let press = theme::Press::track(ui, &resp);
    press.wash(ui, rect.shrink(1.0), 11, false);
    if divider {
        ui.painter().hline(rect.x_range().shrink(1.0), rect.bottom(), Stroke::new(1.0, t.divider));
    }
    let text = line(ui, label, theme::regular(15.0), t.text, rect.width() - 64.0);
    ui.painter().galley(egui::pos2(rect.left() + 16.0, rect.center().y - text.size().y / 2.0), text, t.text);
    if let Some(icon) = trailing {
        let color = if icon == "check" { t.accent_text } else { t.text_faint };
        icons::paint(ui, Rect::from_center_size(rect.right_center() - vec2(24.0, 0.0), vec2(20.0, 20.0)), icon, 20.0, color);
    }
    theme::hand(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Phones are compact held either way; tablets and desktop windows are not.
    #[test]
    fn compact_below_six_hundred_points_wide_or_480_tall() {
        assert!(is_compact(vec2(360.0, 780.0)));
        assert!(is_compact(vec2(411.0, 914.0)));
        assert!(is_compact(vec2(599.9, 900.0)));
        assert!(is_compact(vec2(914.0, 411.0)), "a phone held sideways");
        assert!(is_compact(vec2(800.0, 479.9)));
        assert!(!is_compact(vec2(600.0, 800.0)));
        assert!(!is_compact(vec2(1280.0, 800.0)), "a tablet held sideways");
        assert!(!is_compact(vec2(1024.0, 600.0)));
        assert!(!is_compact(vec2(f32::NAN, f32::NAN)), "an unknown size keeps the desktop layout");
        assert!(!is_compact(vec2(f32::INFINITY, 900.0)));
        assert!(is_compact(vec2(f32::NAN, 411.0)), "the known side still counts");
    }

    /// Every catalogue tool is under one of the All tools page's families.
    #[test]
    fn every_tool_has_a_family() {
        for g in TOOL_GROUPS {
            assert_ne!(Category::of(g.id), Category::All, "{}", g.id);
        }
        for id in HOME_TOOLS {
            assert!(catalog::group(id).is_some(), "the Home grid's {id} is in the catalogue");
        }
    }
}
