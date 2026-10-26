//! Settings, stored as a tiny `islo.ini` next to Islo.exe (portable: nothing is
//! written to C: unless the folder isn't writable, then %APPDATA%\Islo is used).

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

impl Edge {
    pub fn index(self) -> i32 {
        match self {
            Edge::Top => 0,
            Edge::Bottom => 1,
            Edge::Left => 2,
            Edge::Right => 3,
        }
    }
    pub fn from_index(i: i32) -> Edge {
        match i {
            1 => Edge::Bottom,
            2 => Edge::Left,
            3 => Edge::Right,
            _ => Edge::Top,
        }
    }
    pub fn vertical(self) -> bool {
        matches!(self, Edge::Left | Edge::Right)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    // appearance
    pub width: f32, // compact width at rest, 160..300 logical px
    pub edge: Edge,
    pub edge_pos: f32,   // 0..1 along the edge (0.5 = centered)
    pub side_style: i32, // 0 icon tab, 1 thin sliver, 2 full vertical pill
    pub hover_expand: bool,
    pub hover_delay: u32, // ms, 100..1000
    pub theme: i32,       // settings window: 0 follow Windows, 1 light, 2 dark
    pub reduce_motion: bool,
    pub show_on: i32, // 0 main display, 1 all displays, 2 display with the pointer
    pub monitor: i32, // 0 = primary, n = n-th display (used with show_on = 0)
    // at-a-glance info
    pub pill_glance: bool, // idle pill: featured feature left, date + battery right
    pub header_date: bool, // expanded header: date beside the battery
    pub featured: i32,     // left of the idle pill: 0 none, 1 media, 2 timer, 3 pet, 4 focus
    // modules
    pub media: bool,
    pub lyrics: bool,
    pub lyric_lines: i32, // 1..10
    pub hud: bool,        // volume + brightness
    pub hide_flyout: bool,
    pub battery: bool,
    pub shelf: bool,
    pub notifications: bool,
    pub timer: bool,
    pub privacy: bool, // mic / camera / screen recording indicators
    pub focus: bool,
    pub pet: bool,
    pub pet_name: String,
    pub pet_nearby: bool,
    pub pet_peek: bool,  // peek around the island while it's quiet
    pub pet_track: bool, // eyes follow the pointer
    pub pet_skin: i32,   // 0 = Classic (black head, yellow eyes); more skins later
    pub pet_id: u32,  // 24-bit random id used in Bluetooth beacons
    pub pet_fed: u64, // unix seconds, 0 = never
    // full screen & power
    pub hide_fullscreen: bool,
    pub saver_pause: bool,
    // updates
    pub update_check: bool, // look for a new version on GitHub once a day
}

impl Default for Config {
    fn default() -> Self {
        Config {
            width: 200.0,
            edge: Edge::Top,
            edge_pos: 0.5,
            side_style: 0,
            hover_expand: true,
            hover_delay: 300,
            theme: 0,
            reduce_motion: false,
            show_on: 0,
            monitor: 0,
            pill_glance: true,
            header_date: true,
            featured: 3,
            media: true,
            lyrics: true,
            lyric_lines: 3,
            hud: true,
            hide_flyout: true,
            battery: true,
            shelf: true,
            notifications: true,
            timer: true,
            privacy: true,
            focus: true,
            pet: true,
            pet_name: "Mochi".into(),
            pet_nearby: true,
            pet_peek: true,
            pet_track: true,
            pet_skin: 0,
            pet_id: 0,
            pet_fed: 0,
            hide_fullscreen: true,
            saver_pause: true,
            update_check: true,
        }
    }
}

impl Config {
    fn path() -> PathBuf {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let probe = dir.join(".islo-write-test");
                if std::fs::write(&probe, b"").is_ok() {
                    let _ = std::fs::remove_file(&probe);
                    return dir.join("islo.ini");
                }
            }
        }
        let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        let dir = base.join("Islo");
        let _ = std::fs::create_dir_all(&dir);
        dir.join("islo.ini")
    }

    pub fn load() -> Config {
        let mut c = Config::default();
        let Ok(text) = std::fs::read_to_string(Self::path()) else {
            c.ensure_pet_id();
            return c;
        };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            let b = v == "1" || v.eq_ignore_ascii_case("true");
            let f = |d: f32| v.parse::<f32>().unwrap_or(d);
            let i = |d: i32| v.parse::<i32>().unwrap_or(d);
            match k {
                "width" => c.width = f(200.0).clamp(160.0, 300.0),
                "edge" => c.edge = Edge::from_index(i(0)),
                "edge_pos" => c.edge_pos = f(0.5).clamp(0.0, 1.0),
                "side_style" => c.side_style = i(0).clamp(0, 2),
                "hover_expand" => c.hover_expand = b,
                "hover_delay" => c.hover_delay = i(300).clamp(100, 1000) as u32,
                "theme" => c.theme = i(0).clamp(0, 2),
                "reduce_motion" => c.reduce_motion = b,
                "show_on" => c.show_on = i(0).clamp(0, 2),
                "monitor" => c.monitor = i(0).max(0),
                "pill_glance" => c.pill_glance = b,
                "header_date" => c.header_date = b,
                "featured" => c.featured = i(3).clamp(0, 4),
                "media" => c.media = b,
                "lyrics" => c.lyrics = b,
                "lyric_lines" => c.lyric_lines = i(3).clamp(1, 10),
                "hud" => c.hud = b,
                "hide_flyout" => c.hide_flyout = b,
                "battery" => c.battery = b,
                "shelf" => c.shelf = b,
                "notifications" => c.notifications = b,
                "timer" => c.timer = b,
                "privacy" => c.privacy = b,
                "focus" => c.focus = b,
                "pet" => c.pet = b,
                "pet_name" => c.pet_name = v.chars().filter(|c| !c.is_control()).take(24).collect(),
                "pet_nearby" => c.pet_nearby = b,
                "pet_peek" => c.pet_peek = b,
                "pet_track" => c.pet_track = b,
                "pet_skin" => c.pet_skin = i(0).max(0),
                "pet_id" => c.pet_id = v.parse().unwrap_or(0) & 0xff_ffff,
                "pet_fed" => c.pet_fed = v.parse().unwrap_or(0),
                "hide_fullscreen" => c.hide_fullscreen = b,
                "saver_pause" => c.saver_pause = b,
                "update_check" => c.update_check = b,
                _ => {}
            }
        }
        if c.pet_name.trim().is_empty() {
            c.pet_name = "Mochi".into();
        }
        c.ensure_pet_id();
        c
    }

    /// A random-ish 24-bit id for Bluetooth beacons, made once and then saved.
    fn ensure_pet_id(&mut self) {
        if self.pet_id != 0 {
            return;
        }
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(1);
        let mixed = (t as u64 ^ ((std::process::id() as u64) << 20)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        self.pet_id = ((mixed >> 40) as u32 & 0xff_ffff).max(1);
    }

    pub fn save(&self) {
        let b = |v: bool| if v { 1 } else { 0 };
        let text = format!(
            "# Islo settings\n\
             width={}\nedge={}\nedge_pos={:.3}\nside_style={}\nhover_expand={}\nhover_delay={}\ntheme={}\n\
             reduce_motion={}\nshow_on={}\nmonitor={}\npill_glance={}\nheader_date={}\nfeatured={}\n\
             media={}\nlyrics={}\nlyric_lines={}\nhud={}\nhide_flyout={}\nbattery={}\nshelf={}\nnotifications={}\n\
             timer={}\nprivacy={}\nfocus={}\npet={}\npet_name={}\npet_nearby={}\npet_peek={}\npet_track={}\npet_skin={}\npet_id={}\npet_fed={}\nhide_fullscreen={}\nsaver_pause={}\nupdate_check={}\n",
            self.width.round(),
            self.edge.index(),
            self.edge_pos,
            self.side_style,
            b(self.hover_expand),
            self.hover_delay,
            self.theme,
            b(self.reduce_motion),
            self.show_on,
            self.monitor,
            b(self.pill_glance),
            b(self.header_date),
            self.featured,
            b(self.media),
            b(self.lyrics),
            self.lyric_lines,
            b(self.hud),
            b(self.hide_flyout),
            b(self.battery),
            b(self.shelf),
            b(self.notifications),
            b(self.timer),
            b(self.privacy),
            b(self.focus),
            b(self.pet),
            self.pet_name,
            b(self.pet_nearby),
            b(self.pet_peek),
            b(self.pet_track),
            self.pet_skin,
            self.pet_id,
            self.pet_fed,
            b(self.hide_fullscreen),
            b(self.saver_pause),
            b(self.update_check),
        );
        let _ = std::fs::write(Self::path(), text);
    }
}
