//! The compact (phone) layout: below 600 points wide the app swaps the desktop chrome for an app
//! bar and a bottom navigation bar, and side panels for full-width pages and sheets.

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable, by};
use pdfcraft_ui_egui::PdfCraftApp;
use pdfcraft_ui_egui::compact::{Page, Sheet};

/// Two pages and two bookmarks.
const FIXTURE: &[u8] = b"%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R /Outlines 6 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] >> endobj
4 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] >> endobj
6 0 obj << /Type /Outlines /First 7 0 R /Last 8 0 R /Count 2 >> endobj
7 0 obj << /Title (Alpha section) /Parent 6 0 R /Next 8 0 R /Dest [3 0 R /Fit] >> endobj
8 0 obj << /Title (Beta section) /Parent 6 0 R /Prev 7 0 R /Dest [4 0 R /Fit] >> endobj
trailer << /Root 1 0 R >>
%%EOF";

/// A phone held upright: the Android emulator's 411 × 914 dp screen.
const PHONE: egui::Vec2 = egui::vec2(411.0, 914.0);

fn phone(setup: impl FnOnce(&mut PdfCraftApp) + 'static) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(PHONE).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        setup(&mut app);
        app
    });
    // Fonts install on frame 1 and apply on frame 2.
    h.run_steps(4);
    h
}

/// Every piece of text painted on the last frame, with where it was drawn.
fn painted_text(h: &Harness<'static, PdfCraftApp>) -> Vec<(String, egui::Rect)> {
    h.output()
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) => Some((t.galley.text().to_owned(), t.visual_bounding_rect())),
            _ => None,
        })
        .collect()
}

fn text_top(h: &Harness<'static, PdfCraftApp>, text: &str) -> f32 {
    painted_text(h).iter().find(|(s, _)| s == text).unwrap_or_else(|| panic!("{text:?} is painted")).1.top()
}

/// A finger swipe as egui-winit reports it on Android: a touch event for each phase, with the
/// emulated pointer press, moves and release alongside.
fn swipe(h: &mut Harness<'static, PdfCraftApp>, from: egui::Pos2, to: egui::Pos2) {
    let device_id = egui::TouchDeviceId(1);
    let id = egui::TouchId(0);
    let touch = |phase, pos| egui::Event::Touch { device_id, id, phase, pos, force: None };
    h.event(touch(egui::TouchPhase::Start, from));
    h.event(egui::Event::PointerMoved(from));
    h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    let steps = 12;
    for i in 1..=steps {
        let pos = from + (to - from) * (i as f32 / steps as f32);
        h.event(touch(egui::TouchPhase::Move, pos));
        h.event(egui::Event::PointerMoved(pos));
        h.step();
    }
    h.event(touch(egui::TouchPhase::End, to));
    h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.event(egui::Event::PointerGone);
    h.step();
}

#[test]
fn a_touch_swipe_scrolls_the_home_page() {
    let mut h = phone(|app| {
        for i in 0..12 {
            let name = format!("report {i}.pdf");
            app.recent.push(pdfcraft_ui_egui::RecentFile { path: format!("/nowhere/{name}"), name, pages: 3, size: 1024 });
        }
    });
    // Starting on a recent file: the rows take taps, and leave drags to the page.
    let before = text_top(&h, "report 6.pdf");
    swipe(&mut h, egui::pos2(200.0, 760.0), egui::pos2(200.0, 520.0));
    h.run_steps(2);
    let after = text_top(&h, "report 6.pdf");
    assert!(after < before - 150.0, "the page follows the finger up: {before} → {after}");
    assert!(h.state().active.is_none() && h.state().compact.page == Page::Home, "a swipe is not a tap");
}

/// A drag with a pointer that never reports touches (the emulator's mouse, a stylus) scrolls the
/// phone's pages too, even when it starts on text.
#[test]
fn a_mouse_drag_scrolls_the_phone_pages() {
    let mut h = phone(|app| {
        for i in 0..12 {
            let name = format!("report {i}.pdf");
            app.recent.push(pdfcraft_ui_egui::RecentFile { path: format!("/nowhere/{name}"), name, pages: 3, size: 1024 });
        }
    });
    let drag = |h: &mut Harness<'static, PdfCraftApp>, from: egui::Pos2, to: egui::Pos2| {
        h.hover_at(from);
        h.drag_at(from);
        h.step();
        for i in 1..=12 {
            h.hover_at(from + (to - from) * (i as f32 / 12.0));
            h.step();
        }
        h.drop_at(to);
        h.run_steps(2);
    };
    let before = text_top(&h, "report 6.pdf");
    let recent = text_top(&h, "Recent");
    drag(&mut h, egui::pos2(60.0, recent + 4.0), egui::pos2(60.0, recent - 236.0));
    assert!(text_top(&h, "report 6.pdf") < before - 150.0, "the Home page follows the pointer");
    nav_item(&h, "Tools").click();
    h.run_steps(3);
    let before = text_top(&h, "Prepare a form");
    drag(&mut h, egui::pos2(200.0, 700.0), egui::pos2(200.0, 450.0));
    assert!(text_top(&h, "Prepare a form") < before - 150.0, "so does All tools");
}

/// The same swipe on the desktop Home (a touch-screen laptop) scrolls it too.
#[test]
fn a_touch_swipe_scrolls_the_desktop_home_page() {
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        for i in 0..20 {
            let name = format!("report {i}.pdf");
            app.recent.push(pdfcraft_ui_egui::RecentFile { path: format!("/nowhere/{name}"), name, pages: 3, size: 1024 });
        }
        app
    });
    h.run_steps(4);
    let before = text_top(&h, "Recent");
    swipe(&mut h, egui::pos2(600.0, 600.0), egui::pos2(600.0, 250.0));
    h.run_steps(2);
    let after = text_top(&h, "Recent");
    assert!(after < before - 200.0, "the page follows the finger up: {before} → {after}");
}

/// The bottom bar's button called `label` (other widgets may share its name).
fn nav_item<'a>(h: &'a Harness<'static, PdfCraftApp>, label: &'a str) -> egui_kittest::Node<'a> {
    h.query_all_by_label(label).find(|n| n.rect().top() > PHONE.y - 80.0).unwrap_or_else(|| panic!("{label} is in the bottom bar"))
}

/// The button called `label`, not a heading of the same name.
fn button<'a>(h: &'a Harness<'static, PdfCraftApp>, label: &'a str) -> egui_kittest::Node<'a> {
    h.query_all_by_label(label).find(|n| n.accesskit_node().role() == egui::accesskit::Role::Button).unwrap_or_else(|| panic!("a {label} button"))
}

fn with_recent(app: &mut PdfCraftApp) {
    for name in ["Lease agreement 2026 with a name far too long for a phone screen.pdf", "Tax return form.pdf"] {
        app.recent.push(pdfcraft_ui_egui::RecentFile { name: name.into(), path: format!("/nowhere/{name}"), pages: 4, size: 2048 });
    }
}

/// Every widget's left and right edges, by its accessible name.
fn widget_spans(h: &Harness<'static, PdfCraftApp>) -> Vec<(String, f32, f32)> {
    h.query_all(by())
        .filter_map(|n| {
            let node = n.accesskit_node();
            let rect = node.bounding_box()?;
            let ppp = f64::from(h.ctx.pixels_per_point());
            Some((node.label().unwrap_or_default(), (rect.x0 / ppp) as f32, (rect.x1 / ppp) as f32))
        })
        .collect()
}

fn assert_fits(h: &Harness<'static, PdfCraftApp>) {
    for (label, left, right) in widget_spans(h) {
        assert!(left >= -0.5 && right <= PHONE.x + 0.5, "{label:?} spans {left}..{right}, off a {}-point screen", PHONE.x);
    }
    for (text, rect) in painted_text(h).into_iter().filter(|(s, _)| !s.trim().is_empty()) {
        assert!(rect.left() >= -0.5 && rect.right() <= PHONE.x + 0.5, "{text:?} is painted at {rect:?}, off the screen");
    }
}

#[test]
fn the_phone_home_is_full_width_with_no_side_panel() {
    let h = phone(with_recent);
    // The desktop's All tools side panel (its Close button) and tab strip are gone.
    assert!(h.query_by_label("Close panel").is_none(), "no docked tools panel");
    assert!(h.query_by_label("Keyboard shortcuts").is_none(), "no desktop tab strip");
    assert!(h.query_by_label("Menu").is_none(), "no desktop mode bar");
    // The design's home: Open file and Create, the tool grid, All tools, Recent.
    let open = h.get_by_label("Open file").rect();
    assert!(open.width() > 200.0, "Open file takes most of the width: {open:?}");
    h.get_by_label("Create");
    h.get_by_label("All tools");
    for tool in ["Edit a PDF", "Fill & Sign", "Organize pages", "Protect a PDF"] {
        let tile = h.get_by_label(tool).rect();
        assert!(tile.width() >= 80.0 && tile.height() >= 48.0, "{tool} is a full touch target: {tile:?}");
    }
    h.get_by_label("Tax return form.pdf");
    assert_fits(&h);
}

#[test]
fn the_bottom_bar_leads_to_tools_and_settings() {
    let mut h = phone(|_| {});
    for item in ["Home", "Tools", "Settings"] {
        let bar = nav_item(&h, item).rect();
        assert!(bar.bottom() > PHONE.y - 4.0 && bar.height() >= 48.0, "{item} sits in the bottom bar: {bar:?}");
    }
    nav_item(&h, "Tools").click();
    h.run_steps(3);
    assert_eq!(h.state().compact.page, Page::Tools);
    // The All tools page: every family, a row per tool, and the filter chips.
    assert!(h.query_all_by_label("Find tools and commands").any(|n| n.rect().width() > 300.0), "a full-width search field");
    h.get_by_label("Scan & OCR");
    h.get_by_label("Use JavaScript");
    assert_fits(&h);
    button(&h, "Convert").click();
    h.run_steps(3);
    h.get_by_label("Export a PDF");
    assert!(h.query_by_label("Use JavaScript").is_none(), "the Convert chip leaves the other families out");
    h.get_by_label("Export a PDF").click();
    h.run_steps(3);
    assert_eq!(h.state().compact.sheet, Some(Sheet::Tool), "a tool's options open full screen");
    assert_fits(&h);
    nav_item(&h, "Settings").click();
    h.run_steps(3);
    assert_eq!(h.state().compact.page, Page::Settings);
    h.get_by_label("Dark gray").click();
    h.run_steps(3);
    assert_eq!(h.state().theme_preference, pdfcraft_ui_egui::theme::ThemePreference::Dark);
    assert_fits(&h);
}

#[test]
fn the_phone_shows_no_keyboard_shortcuts() {
    let shortcut = |h: &Harness<'static, PdfCraftApp>| painted_text(h).into_iter().find(|(s, _)| s.contains("Ctrl+"));
    // The desktop search field names its shortcut; the phone has no keyboard to press it on.
    let desktop = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app
    });
    let mut desktop = desktop;
    desktop.run_steps(4);
    assert!(shortcut(&desktop).is_some(), "the desktop shows Ctrl+K");
    let mut h = phone(|app| {
        app.open_bytes("lease.pdf", None, FIXTURE.to_vec()).expect("fixture opens");
    });
    assert_eq!(shortcut(&h), None);
    h.get_by_label("More options").click();
    h.run_steps(3);
    h.get_by_label("Save");
    h.get_by_label("Print…");
    h.query_all(by()).find(|n| n.accesskit_node().label().is_some_and(|l| l.starts_with("File") && l.ends_with('⏵'))).expect("the File menu").hover();
    h.run_steps(3);
    assert_eq!(shortcut(&h), None, "nor do the menus");
}

#[test]
fn a_document_gets_the_phone_app_bar_and_mode_bar() {
    let mut h = phone(|app| {
        app.open_bytes("lease.pdf", None, FIXTURE.to_vec()).expect("fixture opens");
    });
    h.run_steps(4);
    assert!(painted_text(&h).iter().any(|(s, _)| s == "lease.pdf"), "the app bar names the document");
    assert!(painted_text(&h).iter().any(|(s, _)| s == "2 pages"));
    for item in ["Read", "Comment", "Fill & Sign", "Pages", "Tools"] {
        let bar = nav_item(&h, item).rect();
        assert!(bar.bottom() > PHONE.y - 4.0, "{item} sits in the bottom bar: {bar:?}");
    }
    assert!(h.query_by_label("Page thumbnails").is_none(), "no desktop rail");
    assert_fits(&h);

    // The side panels open as a full-width sheet over the bottom of the page.
    h.get_by_label("Side panels").click();
    h.run_steps(4);
    assert_eq!(h.state().compact.sheet, Some(Sheet::Contents));
    let bookmark = h.get_by_label("Alpha section").rect();
    assert!(bookmark.left() < 60.0 && bookmark.top() > PHONE.y * 0.25, "the bookmarks fill a bottom sheet: {bookmark:?}");
    assert_fits(&h);
    h.get_by_label("Comments").click();
    h.run_steps(3);
    assert_eq!(h.state().right, Some(pdfcraft_ui_egui::RightPanel::Comments));

    // Comment takes up the commenting tools, in a sheet in place of the side panels.
    nav_item(&h, "Comment").click();
    h.run_steps(3);
    assert_eq!(h.state().compact.sheet, Some(Sheet::Tool));
    assert_eq!(h.state().left, pdfcraft_ui_egui::LeftPanel::Tool("comment"));
    nav_item(&h, "Read").click();
    h.run_steps(3);
    assert_eq!(h.state().compact.sheet, None);

    // Back goes Home, and the document stays open.
    h.get_by_label("Home").click();
    h.run_steps(3);
    assert_eq!(h.state().active, None);
    assert_eq!(h.state().views.len(), 1);
}

#[test]
fn a_wide_window_keeps_the_desktop_layout() {
    let mut h = Harness::builder().with_size(egui::vec2(600.0, 800.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app
    });
    h.run_steps(4);
    h.get_by_label("Menu");
    h.get_by_label("Keyboard shortcuts");
    assert!(h.query_by_label("Settings").is_none(), "no bottom bar at 600 points");
}

#[test]
fn about_fits_the_phone_screen() {
    let mut h = phone(|_| {});
    nav_item(&h, "Settings").click();
    h.run_steps(2);
    h.get_by_label("About PdfCraft").click();
    h.run_steps(4);
    h.get_by_label("This Android app on GitHub");
    assert_fits(&h);
}

#[test]
fn a_phone_held_sideways_keeps_the_phone_layout() {
    let mut h = Harness::builder().with_size(egui::vec2(914.0, 411.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app
    });
    h.run_steps(4);
    assert!(h.query_all_by_label("Settings").any(|n| n.rect().top() > 411.0 - 80.0), "the bottom bar is there");
    assert!(h.query_by_label("Menu").is_none(), "no desktop tab strip on a sideways phone");
}

/// `PDFCRAFT_PHONE_SHOTS=dir` saves the phone layout's screens there, at the emulator's density.
#[test]
fn phone_screenshots() {
    let Ok(dir) = std::env::var("PDFCRAFT_PHONE_SHOTS") else { return };
    let shoot = |h: &mut Harness<'static, PdfCraftApp>, name: &str| {
        h.run_steps(6);
        h.render().unwrap().save(std::path::Path::new(&dir).join(format!("{name}.png"))).unwrap();
    };
    let build = |setup: fn(&mut PdfCraftApp)| {
        let mut h = Harness::builder().with_size(PHONE).with_pixels_per_point(2.0).build_eframe(move |_cc| {
            let mut app = PdfCraftApp::new();
            app.set_option("language", "en").unwrap();
            setup(&mut app);
            app
        });
        h.run_steps(4);
        h
    };
    let mut h = build(with_recent);
    shoot(&mut h, "home");
    nav_item(&h, "Tools").click();
    shoot(&mut h, "tools");
    nav_item(&h, "Settings").click();
    shoot(&mut h, "settings");
    h.get_by_label("About PdfCraft").click();
    shoot(&mut h, "about");
    let mut h = build(|app| {
        let form = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/form.pdf")).unwrap();
        app.open_bytes("form.pdf", None, form).unwrap();
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while h.state().render_pending() && std::time::Instant::now() < deadline {
        h.run_steps(2);
    }
    shoot(&mut h, "document");
    h.get_by_label("More options").click();
    shoot(&mut h, "document-menu");
    h.key_press(egui::Key::Escape);
    h.get_by_label("Side panels").click();
    shoot(&mut h, "document-contents");
    nav_item(&h, "Fill & Sign").click();
    shoot(&mut h, "document-fill-sign");
}
