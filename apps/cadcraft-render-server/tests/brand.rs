//! The brand test: with the host's setup (`cadcraft_render_server::host`), what the interface
//! paints and what its menus offer name the host's product, never CADCraft or ArtCraft. The
//! drawing area is drawn on the CPU here (no device), the rest exactly as the render server lays
//! it out.

use cadcraft_engine::Session;
use cadcraft_render_server::host;
use cadcraft_ui_egui::{CadApp, Services, i18n, menus};

const UPSTREAM: [&str; 2] = ["CADCraft", "ArtCraft"];

fn app(lang: &str) -> CadApp {
    host::name_product();
    let mut app = CadApp::new(Session::empty(), Services::default());
    host::editor(&mut app, lang, true);
    app.session.new_drawing(true);
    app
}

/// The texts one frame paints.
fn painted(app: &mut CadApp, ctx: &egui::Context) -> Vec<String> {
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
        ..Default::default()
    };
    let out = ctx.run_ui(raw, |ui| {
        app.guarded_logic(ui.ctx());
        app.guarded_ui(ui);
    });
    let mut texts = Vec::new();
    for clipped in &out.shapes {
        collect(&clipped.shape, &mut texts);
    }
    texts
}

fn collect(shape: &egui::Shape, out: &mut Vec<String>) {
    match shape {
        egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
        egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
        _ => {}
    }
}

fn upstream_named(texts: &[String]) -> Vec<&String> {
    texts.iter().filter(|t| UPSTREAM.iter().any(|u| t.contains(u))).collect()
}

#[test]
fn the_editor_and_its_about_window_name_the_host_product() {
    for lang in ["en", "zh", "de"] {
        let mut app = app(lang);
        let ctx = egui::Context::default();
        let mut seen = Vec::new();
        for _ in 0..3 {
            seen.extend(painted(&mut app, &ctx));
        }
        app.ui.dialog = Some("about".into());
        for _ in 0..3 {
            seen.extend(painted(&mut app, &ctx));
        }
        assert!(seen.iter().any(|t| t.contains(host::PRODUCT)), "{lang}: the About window names {}", host::PRODUCT);
        assert_eq!(upstream_named(&seen), Vec::<&String>::new(), "{lang}");
    }
}

#[test]
fn the_menus_name_the_host_product_and_offer_no_update_check() {
    let app = app("en");
    let mut labels = Vec::new();
    let mut ids = Vec::new();
    fn walk(entries: &[menus::Entry], labels: &mut Vec<String>, ids: &mut Vec<String>) {
        for e in entries {
            match e {
                menus::Entry::Item { label, id, .. } => {
                    labels.push(label.clone());
                    ids.push(id.clone());
                }
                menus::Entry::Sub { label, children } => {
                    labels.push(label.clone());
                    walk(children, labels, ids);
                }
            }
        }
    }
    for (top, entries) in menus::tree(&app) {
        labels.push(top);
        walk(&entries, &mut labels, &mut ids);
    }
    for lang in ["en", "zh", "fr", "ko"] {
        let shown: Vec<String> = i18n::with_language(i18n::locale_language(lang).unwrap_or("en"), || labels.iter().map(|l| i18n::menu(l).to_string()).collect());
        assert_eq!(upstream_named(&shown), Vec::<&String>::new(), "{lang}");
        assert!(shown.iter().any(|l| l.contains(host::PRODUCT)), "{lang}: Help names {}", host::PRODUCT);
    }
    assert!(!ids.iter().any(|id| id == "ui.updates.check"), "no update check of CADCraft's own");
}

#[test]
fn messages_on_the_command_and_status_lines_name_the_host_product() {
    let mut app = app("en");
    app.session.echo("CADCraft views are 2D.");
    app.set_status("CADCraft can't save back to it");
    assert_eq!(app.session.log.last().map(String::as_str), Some("CadZ views are 2D."));
    assert_eq!(app.status.as_ref().map(|s| s.0.as_str()), Some("CadZ can't save back to it"));
}
