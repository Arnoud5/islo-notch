//! UI snapshot tool (non-Windows only). Renders island states and the settings
//! window to PNG files with the same software renderer the app uses on Windows,
//! so the visuals can be reviewed on any machine:  `cargo run -- out_dir`

use crate::{Island, SettingsWindow, ShelfItem};
use slint::platform::software_renderer::{MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::rc::Rc;

thread_local! {
    static WINDOWS: std::cell::RefCell<Vec<Rc<MinimalSoftwareWindow>>> = Default::default();
    static STRESS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

struct SnapPlatform;
impl Platform for SnapPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        let mode = if STRESS.with(|s| s.get()) { RepaintBufferType::ReusedBuffer } else { RepaintBufferType::NewBuffer };
        let w = MinimalSoftwareWindow::new(mode);
        WINDOWS.with(|v| v.borrow_mut().push(w.clone()));
        Ok(w)
    }
}

fn render(win: &MinimalSoftwareWindow, w: u32, h: u32, scale: f32, path: &str, bg: [u8; 3]) {
    win.dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: scale });
    let (pw, ph) = ((w as f32 * scale) as u32, (h as f32 * scale) as u32);
    win.set_size(slint::PhysicalSize::new(pw, ph));
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
    }
    std::thread::sleep(std::time::Duration::from_millis(450));
    slint::platform::update_timers_and_animations();
    win.request_redraw();
    let mut buf = vec![PremultipliedRgbaColor::default(); (pw * ph) as usize];
    win.draw_if_needed(|r| {
        r.render(&mut buf, pw as usize);
    });
    let mut out = Vec::with_capacity(buf.len() * 3);
    for p in &buf {
        let ia = 255 - p.alpha as u16;
        out.push((p.red as u16 + bg[0] as u16 * ia / 255) as u8);
        out.push((p.green as u16 + bg[1] as u16 * ia / 255) as u8);
        out.push((p.blue as u16 + bg[2] as u16 * ia / 255) as u8);
    }
    let f = std::fs::File::create(path).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), pw, ph);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&out).unwrap();
    println!("wrote {path}");
}

fn art() -> slint::Image {
    let mut px = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(96, 96);
    for (i, p) in px.make_mut_slice().iter_mut().enumerate() {
        let (x, y) = ((i % 96) as u8, (i / 96) as u8);
        *p = slint::Rgba8Pixel { r: 200 - y, g: 60 + x, b: 140 + y / 2, a: 255 };
    }
    slint::Image::from_rgba8(px)
}

fn shape(i: &Island, w: f32, h: f32, r: f32) {
    i.set_pill_w(w);
    i.set_pill_h(h);
    i.set_radius(r);
}

/// Frame-by-frame run of "hover → open lyrics" with partial redraws, like the app.
fn stress() {
    let island = Island::new().unwrap();
    let iw = WINDOWS.with(|v| v.borrow()[0].clone());
    iw.dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: 1.5 });
    let (pw, ph) = (660u32, 465u32);
    iw.set_size(slint::PhysicalSize::new(pw, ph));
    let lines: Vec<slint::SharedString> = (0..80).map(|k| if k % 9 == 4 { "♪".into() } else { format!("Line {k} of the song with some words 夜").into() }).collect();
    island.set_lyrics(Rc::new(VecModel::from(lines.clone())).into());
    island.set_has_media(true);
    island.set_playing(true);
    island.set_track_title("Song".into());
    let mut buf = vec![PremultipliedRgbaColor::default(); (pw * ph) as usize];
    let mut h = 36.0f32;
    for frame in 0..400 {
        if frame == 5 {
            island.set_mode(2);
            island.set_panel(4);
            island.set_panels(ModelRc::new(VecModel::from(vec![4, 0, 3, 5])));
        }
        if frame > 5 && h < 294.0 { h = (h + 9.0).min(300.0); }
        island.set_pill_w(380.0);
        island.set_pill_h(h);
        island.set_radius(30.0);
        if frame % 20 == 0 {
            let idx = (frame / 20) as i32;
            island.set_lyric_index(idx);
            island.set_lyric_scroll(((idx - 2).max(0) * 22) as f32);
            let win: Vec<slint::SharedString> = lines.iter().skip(idx as usize).take(3).cloned().collect();
            island.set_lyric_window(ModelRc::new(VecModel::from(win)));
        }
        island.set_wave(ModelRc::new(VecModel::from(vec![(frame % 7) as f32 / 7.0; 5])));
        std::thread::sleep(std::time::Duration::from_millis(8));
        slint::platform::update_timers_and_animations();
        iw.request_redraw();
        iw.draw_if_needed(|r| {
            r.render(&mut buf, pw as usize);
        });
    }
    println!("stress ok");
}

pub fn run() {
    slint::platform::set_platform(Box::new(SnapPlatform)).unwrap();
    if std::env::var("ISLO_CLAMP").is_ok() {
        let island = Island::new().unwrap();
        let iw = WINDOWS.with(|v| v.borrow()[0].clone());
        iw.set_size(slint::PhysicalSize::new(440, 310));
        let mut buf = vec![PremultipliedRgbaColor::default(); 440 * 310];
        for edge in 0..5 {
            island.set_edge(edge);
            island.set_mode(2);
            island.set_panel(4);
            island.set_pill_w(380.0);
            island.set_pill_h(298.0);
            iw.request_redraw();
            iw.draw_if_needed(|r| { r.render(&mut buf, 440); });
            println!("edge {edge} ok");
        }
        return;
    }
    if std::env::var("ISLO_STRESS").is_ok() {
        // same repaint mode as the Windows backend
        STRESS.with(|s| s.set(true));
        return stress();
    }
    let out = std::env::args().nth(1).unwrap_or_else(|| "snapshots".into());
    std::fs::create_dir_all(&out).unwrap();
    let island = Island::new().unwrap();
    let settings = SettingsWindow::new().unwrap();
    let (iw, sw) = WINDOWS.with(|v| (v.borrow()[0].clone(), v.borrow()[1].clone()));
    island.set_reduce_motion(true);
    let bg = [42, 42, 42];
    let (w, h) = (440, 236);
    let i = &island;

    shape(i, 200.0, 36.0, 14.0);
    render(&iw, w, h, 2.0, &format!("{out}/01-idle.png"), bg);

    // at a glance: featured left, date + battery right
    i.set_date_text("Mon 26 Oct".into()); i.set_has_battery(true); i.set_battery_level(82);
    i.set_compact_kind(19); shape(i, 236.0, 36.0, 14.0);
    for (n, f) in [("40-glance-pet", 3), ("41-glance-media", 1), ("42-glance-timer", 2), ("43-glance-focus", 4)] {
        i.set_featured(f);
        render(&iw, w, h, 2.0, &format!("{out}/{n}.png"), bg);
    }
    i.set_featured(0); shape(i, 200.0, 36.0, 14.0);
    render(&iw, w, h, 2.0, &format!("{out}/44-glance-none.png"), bg);
    i.set_has_battery(false);
    render(&iw, w, h, 2.0, &format!("{out}/45-glance-desktop.png"), bg);
    // Mochi peeking (eyes following a pointer down-right), and the bell badge
    i.set_has_battery(true); i.set_featured(3); i.set_pet_awake(true); shape(i, 236.0, 36.0, 14.0);
    {
        use slint::ComponentHandle;
        let g = i.global::<crate::PetSkin>();
        g.set_look_x(0.7); g.set_look_y(0.5);
    }
    for (n, side, pos) in [("46-peek-below", 1, 0.3), ("47-peek-left", 2, 0.5), ("48-peek-right", 3, 0.5)] {
        i.set_peek_side(side); i.set_peek_pos(pos); i.set_peek_out(true);
        render(&iw, w, h, 2.0, &format!("{out}/{n}.png"), bg);
    }
    i.set_peek_side(0); i.set_peek_out(false);
    i.set_note_count(3); shape(i, 270.0, 36.0, 14.0);
    render(&iw, w, h, 2.0, &format!("{out}/49-glance-badge.png"), bg);
    i.set_pet_awake(false);
    render(&iw, w, h, 2.0, &format!("{out}/50-glance-asleep.png"), bg);
    i.set_note_count(0);
    i.set_featured(3); i.set_compact_kind(0);

    i.set_has_media(true); i.set_playing(true); i.set_has_art(true); i.set_art(art());
    i.set_accent(slint::Color::from_rgb_u8(0xfc, 0x90, 0xbc));
    i.set_track_title("Midnight Drive".into()); i.set_subtitle("Lofi Collective · Spotify".into());
    i.set_wave(ModelRc::new(VecModel::from(vec![0.4, 0.9, 0.6, 1.0, 0.5])));
    i.set_compact_kind(1); shape(i, 260.0, 36.0, 14.0);
    render(&iw, w, h, 2.0, &format!("{out}/02-compact-media.png"), bg);

    i.set_mode(1); i.set_hud_kind(0); i.set_hud_value(0.64); shape(i, 260.0, 36.0, 14.0);
    render(&iw, w, h, 2.0, &format!("{out}/03-hud-volume.png"), bg);
    i.set_hud_kind(1); i.set_hud_value(0.4);
    render(&iw, w, h, 2.0, &format!("{out}/04-hud-brightness.png"), bg);

    i.set_mode(0); i.set_compact_kind(5); i.set_battery_level(82); i.set_charging(true); shape(i, 220.0, 36.0, 14.0);
    render(&iw, w, h, 2.0, &format!("{out}/05-charging.png"), bg);
    i.set_compact_kind(6); i.set_battery_level(14); i.set_charging(false);
    render(&iw, w, h, 2.0, &format!("{out}/06-low.png"), bg);

    let items: Vec<ShelfItem> = [("Q3-report.pdf", "2.4 MB"), ("hero-shot.png", "860 KB"), ("islo-spike.zip", "14.0 MB")].iter()
        .map(|(n, s)| ShelfItem { name: (*n).into(), size: (*s).into(), icon: art(), has_icon: n.ends_with("png") }).collect();
    i.set_shelf_items(Rc::new(VecModel::from(items)).into());
    i.set_shelf_summary("3 files · 17.3 MB".into());
    i.set_compact_kind(2); shape(i, 200.0, 36.0, 14.0);
    render(&iw, w, h, 2.0, &format!("{out}/07-shelf-waiting.png"), bg);
    i.set_compact_kind(4); i.set_kept_text("1 file".into());
    render(&iw, w, h, 2.0, &format!("{out}/08-kept.png"), bg);
    i.set_compact_kind(7); shape(i, 260.0, 36.0, 14.0);
    render(&iw, w, h, 2.0, &format!("{out}/09-error.png"), bg);

    i.set_battery_level(82); i.set_has_battery(true);
    i.set_mode(2); i.set_panel(0); i.set_panels(ModelRc::new(VecModel::from(vec![0])));
    i.set_has_timeline(true); i.set_progress(0.32); i.set_elapsed("1:12".into()); i.set_duration("3:48".into());
    shape(i, 380.0, 162.0, 30.0);
    render(&iw, w, h, 2.0, &format!("{out}/10-expanded-media.png"), bg);
    i.set_panels(ModelRc::new(VecModel::from(vec![0, 1])));
    shape(i, 380.0, 196.0, 30.0);
    render(&iw, w, h, 2.0, &format!("{out}/11-expanded-header.png"), bg);
    i.set_panel(1); shape(i, 380.0, 210.0, 30.0);
    render(&iw, w, h, 2.0, &format!("{out}/12-shelf-files.png"), bg);
    i.set_drag_hover(true); i.set_panels(ModelRc::new(VecModel::from(vec![0]))); shape(i, 380.0, 152.0, 30.0);
    render(&iw, w, h, 2.0, &format!("{out}/13-drop-zone.png"), bg);
    i.set_drag_hover(false);
    i.set_edge(1); i.set_mode(0); i.set_compact_kind(1); shape(i, 260.0, 36.0, 14.0);
    render(&iw, w, h, 2.0, &format!("{out}/14-bottom-edge.png"), bg);

    i.set_edge(0);
    i.set_timer_text("4:59".into()); i.set_timer_left(0.7); i.set_timer_label("5 min".into()); i.set_timer_running(true);
    i.set_compact_kind(8); shape(i, 200.0, 36.0, 14.0);
    render(&iw, w, h, 2.0, &format!("{out}/18-timer.png"), bg);
    i.set_compact_kind(9); i.set_mic_app("Discord".into()); shape(i, 220.0, 36.0, 14.0);
    render(&iw, w, h, 2.0, &format!("{out}/19-mic.png"), bg);
    i.set_compact_kind(11); i.set_focus_text("On".into());
    render(&iw, w, h, 2.0, &format!("{out}/20-focus.png"), bg);
    i.set_mode(2); i.set_panel(3); i.set_panels(ModelRc::new(VecModel::from(vec![0, 3]))); shape(i, 380.0, 162.0, 30.0);
    render(&iw, w, h, 2.0, &format!("{out}/21-timer-panel.png"), bg);
    i.set_timer_running(false);
    render(&iw, w, h, 2.0, &format!("{out}/22-timer-start.png"), bg);
    i.set_edge(2); i.set_mode(0); i.set_tab(true); i.set_anchor(118.0);
    i.set_tab_glyphs(ModelRc::new(VecModel::from(vec![0, 4, 1]))); i.set_tab_extra(1); shape(i, 36.0, 148.0, 14.0);
    render(&iw, w, h, 2.0, &format!("{out}/23-side-tab.png"), bg);
    i.set_tab(false); i.set_edge(0);
    let lines: Vec<slint::SharedString> = ["Streetlights blinking out of time", "Radio humming something low", "Every exit looks like home", "Windows down, the city glows", "We don't need to know the way", "Just the road and what we say", "Hold the wheel and hold the night", "♪", "Counting signs in amber light 夜の道 🎵", "If the morning finds us here", "Let it find us singing clear"].iter().map(|x| (*x).into()).collect();
    i.set_lyrics(Rc::new(VecModel::from(lines.clone())).into());
    i.set_lyric_window(Rc::new(VecModel::from(lines[2..5].to_vec())).into());
    i.set_lyric_index(2);
    i.set_mode(0); i.set_compact_kind(13); shape(i, 380.0, 76.0, 30.0);
    render(&iw, w, 310, 2.0, &format!("{out}/24-lyrics-compact.png"), bg);
    i.set_compact_kind(1); i.set_compact_title("Every exit looks like home".into()); shape(i, 380.0, 36.0, 14.0);
    render(&iw, w, 310, 2.0, &format!("{out}/25-lyrics-1line.png"), bg);
    i.set_mode(2); i.set_panel(4); i.set_panels(ModelRc::new(VecModel::from(vec![4, 0, 3]))); shape(i, 380.0, 294.0, 30.0);
    render(&iw, w, 310, 2.0, &format!("{out}/26-lyrics-panel.png"), bg);

    i.set_mode(0); i.set_compact_kind(14); i.set_pet_name("Mochi".into()); shape(i, 220.0, 36.0, 14.0);
    render(&iw, w, h, 2.0, &format!("{out}/27-pet-asleep.png"), bg);
    i.set_visitor("Bento".into()); i.set_visitor_owner("Rania".into());
    render(&iw, w, h, 2.0, &format!("{out}/28-pet-visitor.png"), bg);
    i.set_visitor("".into());
    i.set_mode(2); i.set_panel(5); i.set_panels(ModelRc::new(VecModel::from(vec![0, 3, 5])));
    i.set_pet_status("Happy · fed 2 hours ago".into()); i.set_pet_awake(true);
    shape(i, 380.0, 158.0, 30.0);
    render(&iw, w, h, 2.0, &format!("{out}/29-pet-panel.png"), bg);
    i.set_pet_list_open(true);
    i.set_nearby(ModelRc::new(VecModel::from(vec![crate::NearbyItem { pet: "Bento".into(), owner: "Rania".into() }])));
    shape(i, 380.0, 210.0, 30.0);
    render(&iw, w, h, 2.0, &format!("{out}/30-pet-send.png"), bg);
    i.set_pet_list_open(false);

    i.set_mode(0); i.set_compact_kind(18); i.set_note_app("Slack".into()); i.set_note_title("Rania: are we still on for 3?".into()); shape(i, 260.0, 36.0, 14.0);
    render(&iw, w, h, 2.0, &format!("{out}/31-note-new.png"), bg);
    i.set_compact_kind(16); i.set_note_count(2); shape(i, 220.0, 36.0, 14.0);
    render(&iw, w, h, 2.0, &format!("{out}/32-note-count.png"), bg);
    i.set_compact_kind(17); i.set_call_name("Rania Putri".into()); shape(i, 260.0, 36.0, 14.0);
    render(&iw, w, h, 2.0, &format!("{out}/33-call.png"), bg);
    i.set_notes(ModelRc::new(VecModel::from(vec![
        crate::NoteItem { app: "Snipping Tool".into(), initial: "S".into(), title: "Screenshot copied to clipboard".into(), body: "Automatically saved to screenshots folder.".into(), time: "now".into(), call: false },
        crate::NoteItem { app: "WhatsApp".into(), initial: "W".into(), title: "Patient Journey 7.0".into(), body: "~ glenn: mas @~Luthfi M Nabil boleh bantu cek ga OPA2610260016 appointment_id: \"6c9ac858-746f-43b0-aab3-016655\"".into(), time: "2 min".into(), call: false },
        crate::NoteItem { app: "WhatsApp".into(), initial: "W".into(), title: "Nopia".into(), body: "hope_id: -795813 https://jam.dev/c/bb60bbe0-ad61-451b-8b9e-d7db cc @Mas Kiiay Rian FO".into(), time: "2 min".into(), call: false },
    ])));
    i.set_mode(2); i.set_panel(6); i.set_panels(ModelRc::new(VecModel::from(vec![4, 0, 3, 6, 5, 2]))); shape(i, 380.0, 32.0 + 34.0 + 232.0, 30.0);
    render(&iw, w, 310, 2.0, &format!("{out}/34-notes-panel.png"), bg);

    i.set_mode(2); i.set_panel(2); i.set_panels(ModelRc::new(VecModel::from(vec![0, 3, 6, 5, 2])));
    i.set_battery_level(56); i.set_charging(true); i.set_power_title("Charging".into()); i.set_power_sub("Plugged in · about 40 min to full".into());
    shape(i, 380.0, 32.0 + 34.0 + 56.0, 30.0);
    render(&iw, w, h, 2.0, &format!("{out}/35-power.png"), bg);
    i.set_charging(false); i.set_battery_level(14); i.set_power_title("Battery low".into()); i.set_power_sub("About 25 min left".into());
    render(&iw, w, h, 2.0, &format!("{out}/36-power-low.png"), bg);
    i.set_battery_level(72); i.set_power_title("On battery".into()); i.set_power_sub("About 3 h 10 min left".into());
    render(&iw, w, h, 2.0, &format!("{out}/37-power-batt.png"), bg);

    for (n, dark, page) in [("15-settings-appearance-dark", true, 1), ("16-settings-modules-light", false, 2), ("17-settings-general-dark", true, 0), ("38-settings-about-light", false, 4), ("39-settings-about-dark", true, 4)] {
        settings.set_dark(dark);
        settings.set_page(page);
        settings.set_memory_text("14.2 MB in use".into());
        render(&sw, 1000, 900, 1.0, &format!("{out}/{n}.png"), if dark { [0, 0, 0] } else { [243, 243, 243] });
    }
    let _ = island.window();
}
