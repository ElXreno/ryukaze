//! The keyboard pictures: each laptop's keys and where they sit, its
//! touchpad and its logo, from TOML files built in and in the XDG data
//! dirs (see docs/keyboard.md#the-picture).

use std::{env, ffi::OsString, fs, path::PathBuf};

use iced::{Point, Rectangle, Size};
use ryukaze_core::{
    layout::{self, PAD_INSET, STRIPE_GAP},
    lighting::Part,
    steelseries,
};

use crate::machine::Machine;

/// The built-in layouts, as files of the same names would be.
const BUILT_IN: [(&str, &str); 2] = [
    (
        "ms-1824.toml",
        include_str!("../../../data/keyboards/ms-1824.toml"),
    ),
    (
        "default.toml",
        include_str!("../../../data/keyboards/default.toml"),
    ),
];

/// A key on the picture: its HID usage, a label, and where it sits, in key
/// widths from the top left.
#[derive(Debug, Clone, PartialEq)]
pub struct Key {
    pub code: u8,
    pub label: String,
    pub rect: Rectangle,
}

#[derive(Debug, Clone, PartialEq)]
struct Touchpad {
    rect: Rectangle,
    rows: u8,
}

/// A keyboard's picture.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub name: String,
    /// The DMI board names and the CHIDs it's for; none, any laptop's.
    boards: Vec<String>,
    chids: Vec<String>,
    pub keys: Vec<Key>,
    touchpad: Option<Touchpad>,
    deck: Option<Rectangle>,
    logo: Option<Logo>,
    /// What the keys span.
    span: Rectangle,
}

/// The logo, where it is on the lid as parts of it, and the lid's shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Logo {
    pub on_lid: Rectangle,
    /// The lid's width to its height.
    pub lid: f32,
}

/// A box of the picture.
fn rect(bounds: layout::Bounds) -> Rectangle {
    Rectangle::new(
        Point::new(bounds.x, bounds.y),
        Size::new(bounds.w, bounds.h),
    )
}

impl Layout {
    /// A layout file's text, checked (see [`layout::File::check`]).
    pub fn parse(text: &str) -> Result<Self, String> {
        let file = layout::File::parse(text)?;
        let keys: Vec<Key> = file
            .keys
            .into_iter()
            .map(|key| Key {
                code: key.hid,
                rect: rect(key.bounds()),
                label: key.label,
            })
            .collect();
        let touchpad = file.touchpad.map(|pad| Touchpad {
            rect: rect(pad.bounds()),
            rows: pad.rows,
        });
        let logo = file.logo.map(|logo| Logo {
            on_lid: rect(logo.bounds()),
            lid: logo.lid,
        });
        let span = keys
            .iter()
            .skip(1)
            .fold(keys[0].rect, |span, key| span.union(&key.rect));
        Ok(Self {
            name: file.name,
            boards: file.boards,
            chids: file.chids,
            keys,
            touchpad,
            deck: file.deck.map(rect),
            logo,
            span,
        })
    }

    /// For the laptop of this DMI board name, or of this CHID.
    fn fits(&self, board: Option<&str>, chid: Option<&str>) -> bool {
        let any = |known: &[String], what: Option<&str>| {
            what.is_some_and(|what| {
                known
                    .iter()
                    .any(|known| known.trim().eq_ignore_ascii_case(what.trim()))
            })
        };
        any(&self.boards, board) || any(&self.chids, chid)
    }

    /// For any laptop: no boards and no CHIDs.
    fn for_any(&self) -> bool {
        self.boards.is_empty() && self.chids.is_empty()
    }

    /// What the keys span.
    pub fn keys_span(&self) -> Rectangle {
        self.span
    }

    /// Where a light beside the keys sits, if the picture has it.
    pub fn part(&self, part: Part) -> Option<Rectangle> {
        match part {
            Part::Bar => self.touchpad.as_ref().map(|pad| pad.rect),
            Part::Logo => self.logo.map(|logo| logo.on_lid),
        }
    }

    /// The laptop's lower half, keys and touchpad on it, if the picture has
    /// it.
    pub fn deck(&self) -> Option<Rectangle> {
        self.deck
    }

    /// The logo on its lid, if the picture has it.
    pub fn logo(&self) -> Option<Logo> {
        self.logo
    }

    /// The touchpad's rows of light, from the keys to the front.
    pub fn stripes(&self) -> Vec<(u8, Rectangle)> {
        let Some(pad) = &self.touchpad else {
            return Vec::new();
        };
        let rect = pad.rect;
        let pitch = (rect.height - 2.0 * PAD_INSET + STRIPE_GAP) / f32::from(pad.rows);
        (0..pad.rows)
            .map(|row| {
                let at = Point::new(
                    rect.x + PAD_INSET,
                    rect.y + PAD_INSET + pitch * f32::from(row),
                );
                let size = Size::new(rect.width - 2.0 * PAD_INSET, pitch - STRIPE_GAP);
                (row, Rectangle::new(at, size))
            })
            .collect()
    }

    /// A point of the picture in the KLC's plane, which the keys span.
    pub fn plane(&self, at: Point) -> [u16; 2] {
        let units = |value: f32, from: f32, keys: f32, plane: u16| {
            ((value - from) / keys * f32::from(plane))
                .round()
                .clamp(0.0, f32::from(u16::MAX)) as u16
        };
        let span = self.span;
        [
            units(at.x, span.x, span.width, steelseries::ACROSS),
            units(at.y, span.y, span.height, steelseries::DOWN),
        ]
    }
}

/// The built-in layout for any laptop.
pub fn default() -> Layout {
    Layout::parse(BUILT_IN[1].1).expect("the built-in default layout is valid")
}

/// The layout for `machine`, from the files there are (see [`dirs`]) or
/// built in.
pub fn for_machine(machine: &Machine) -> Layout {
    let dirs = dirs(
        env::var_os("XDG_DATA_HOME"),
        env::var_os("HOME"),
        env::current_exe().ok(),
        env::var_os("XDG_DATA_DIRS"),
    );
    let chid = machine
        .vendor
        .as_deref()
        .zip(machine.model.as_deref())
        .map(|(vendor, model)| layout::chid(vendor, model));
    pick(load(&dirs), machine.board.as_deref(), chid.as_deref()).unwrap_or_else(default)
}

/// Where layout files are looked for, first first: `ryukaze/keyboards` in
/// `$XDG_DATA_HOME` (`~/.local/share`), then in the `share` beside the
/// program's own `bin`, as installed, then in each of `$XDG_DATA_DIRS`
/// (`/usr/local/share:/usr/share`).
fn dirs(
    data_home: Option<OsString>,
    home: Option<OsString>,
    exe: Option<PathBuf>,
    data_dirs: Option<OsString>,
) -> Vec<PathBuf> {
    let absolute = |path: PathBuf| path.is_absolute().then_some(path);
    let user = data_home
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .and_then(absolute)
        .or_else(|| {
            home.map(PathBuf::from)
                .and_then(absolute)
                .map(|home| home.join(".local/share"))
        });
    let system = data_dirs
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    let beside = exe
        .and_then(absolute)
        .and_then(|exe| Some(exe.parent()?.parent()?.join("share")));
    user.into_iter()
        .chain(beside)
        .chain(env::split_paths(&system).filter_map(absolute))
        .map(|dir| dir.join("ryukaze/keyboards"))
        .collect()
}

/// The `.toml` files in `dirs`, in order and by name within each, then the
/// built-in ones; a file that doesn't read or check out is told of on
/// stderr and left out.
fn load(dirs: &[PathBuf]) -> Vec<Layout> {
    let mut layouts = Vec::new();
    let mut take = |source: &dyn std::fmt::Display, text: Result<String, String>| match text
        .and_then(|text| Layout::parse(&text))
    {
        Ok(layout) => layouts.push(layout),
        Err(err) => eprintln!("ryukaze: keyboard layout {source}: {err}; skipped"),
    };
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(|entry| Some(entry.ok()?.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "toml") && path.is_file())
            .collect();
        files.sort();
        for path in files {
            let text = fs::read_to_string(&path).map_err(|err| err.to_string());
            take(&path.display(), text);
        }
    }
    for (name, text) in BUILT_IN {
        take(&format!("{name} (built in)"), Ok(text.to_owned()));
    }
    layouts
}

/// The first of `layouts` for `board` or `chid`, or else the first for any
/// laptop.
fn pick(mut layouts: Vec<Layout>, board: Option<&str>, chid: Option<&str>) -> Option<Layout> {
    let at = layouts
        .iter()
        .position(|layout| layout.fits(board, chid))
        .or_else(|| layouts.iter().position(Layout::for_any))?;
    Some(layouts.swap_remove(at))
}

#[cfg(test)]
pub fn built_in(name: &str) -> Layout {
    let (_, text) = BUILT_IN
        .iter()
        .find(|(file, _)| *file == name)
        .expect("a built-in layout");
    Layout::parse(text).unwrap()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use ryukaze_core::{keyboard, lightbar};

    use super::*;

    fn key(layout: &Layout, code: u8) -> &Key {
        layout.keys.iter().find(|key| key.code == code).unwrap()
    }

    /// The keys left to right along the row `of` is in.
    fn row_of(layout: &Layout, of: u8) -> Vec<u8> {
        let y = key(layout, of).rect.y;
        let mut row: Vec<&Key> = layout.keys.iter().filter(|key| key.rect.y == y).collect();
        row.sort_by(|a, b| a.rect.x.total_cmp(&b.rect.x));
        row.iter().map(|key| key.code).collect()
    }

    /// Up right above Down, Right under the numpad's first column, the
    /// numpad's 0 one column wide, and the board as wide as measured.
    fn arrows_and_numpad(layout: &Layout, across: f32, down: f32) {
        assert_eq!(layout.keys.len(), 102);
        let (up, below) = (key(layout, 0x52).rect, key(layout, 0x51).rect);
        assert!(
            (up.center_x() - below.center_x()).abs() < 0.05,
            "{up:?} {below:?}"
        );
        assert!(up.y + up.height <= below.y + 0.01);
        let (right, numlock) = (key(layout, 0x4f).rect, key(layout, 0x53).rect);
        assert!(numlock.x <= right.center_x() && right.center_x() <= numlock.x + numlock.width);
        assert!((key(layout, 0x62).rect.width - numlock.width).abs() < 0.02);
        assert_eq!(key(layout, 0xe4).label, "Copilot");
        let span = layout.keys_span();
        assert_eq!((span.x, span.y), (0.0, 0.0));
        assert!((span.width - across).abs() < 0.001 && (span.height - down).abs() < 0.001);
        assert_eq!(layout.plane(Point::ORIGIN), [0, 0]);
        assert_eq!(
            layout.plane(Point::new(span.width, span.height)),
            [steelseries::ACROSS, steelseries::DOWN]
        );
    }

    #[test]
    fn the_built_in_layouts_check_out() {
        for (name, text) in BUILT_IN {
            Layout::parse(text).unwrap_or_else(|err| panic!("{name}: {err}"));
        }
        let alpha = built_in("default.toml");
        assert!(alpha.for_any());
        assert!(alpha.part(Part::Bar).is_none() && alpha.part(Part::Logo).is_none());
        // Every key the config can name.
        for (name, code) in keyboard::names() {
            assert!(alpha.keys.iter().any(|key| key.code == code), "{name}");
        }
    }

    #[test]
    fn the_alphas_keys_as_measured() {
        let alpha = built_in("default.toml");
        arrows_and_numpad(&alpha, 18.919, 5.697);
        assert_eq!(
            row_of(&alpha, 0x51),
            [
                0xe0, 0xe3, 0xe2, 0x2c, 0xe6, 0x64, 0xf0, 0xe4, 0x50, 0x51, 0x4f, 0x62, 0x63
            ]
        );
    }

    #[test]
    fn the_example_in_the_docs_checks_out() {
        let page = include_str!("../../../docs/keyboard.md");
        let (_, picture) = page.split_once("## The picture").unwrap();
        let (_, example) = picture.split_once("```toml\n").unwrap();
        let (example, _) = example.split_once("```").unwrap();
        let layout = Layout::parse(example).unwrap();
        assert!(layout.fits(Some("MS-1234"), None) && layout.part(Part::Bar).is_some());
    }

    #[test]
    fn the_titans_keys_as_measured() {
        let titan = built_in("ms-1824.toml");
        assert!(titan.fits(Some("MS-1824"), None) && titan.fits(Some("ms-1824\n"), None));
        arrows_and_numpad(&titan, 18.481, 5.595);
        let (up, below) = (key(&titan, 0x52), key(&titan, 0x51));
        assert_eq!(up.rect.x, below.rect.x);
        // Right under NumLock and 1.
        let right = key(&titan, 0x4f).rect.x;
        assert_eq!(right, key(&titan, 0x53).rect.x);
        assert_eq!(right, key(&titan, 0x59).rect.x);
        assert_eq!(key(&titan, 0x62).rect.width, 1.0);
        assert_eq!(
            row_of(&titan, 0x51),
            [
                0xe0, 0xe3, 0xe2, 0x2c, 0xe6, 0x64, 0xe4, 0xf0, 0x50, 0x51, 0x4f, 0x62, 0x63
            ]
        );
        // PrtSc over the numpad's first column, after Delete.
        assert_eq!(key(&titan, 0x46).rect.x, key(&titan, 0x53).rect.x);
        let top = row_of(&titan, 0x29);
        assert_eq!(&top[13..17], [0x47, 0x49, 0x4c, 0x46]);
        // The touchpad's rows below the keys, from the keys down.
        assert!(titan.part(Part::Bar).is_some() && titan.part(Part::Logo).is_some());
        let stripes = titan.stripes();
        assert_eq!(stripes.len(), usize::from(lightbar::BAR_ROWS));
        assert!(stripes[0].1.y > titan.keys_span().height);
        assert!(stripes.windows(2).all(|two| two[0].1.y < two[1].1.y));
    }

    #[test]
    fn a_broken_layout_is_refused() {
        let one = |key: &str| format!("name = \"x\"\nkeys = [\n  {key},\n]\n");
        let good = "{ hid = 0x29, x = 0, y = 0, w = 1, h = 1 }";
        assert!(Layout::parse(&one(good)).is_ok());
        for (text, why) in [
            (
                one("{ hid = 0x32, x = 0, y = 0, w = 1, h = 1 }"),
                "no such key",
            ),
            (
                one("{ hid = 0x29, x = 0, y = 0, w = 0, h = 1 }"),
                "positive",
            ),
            (
                one("{ hid = 0x29, x = 0, y = 0, w = 1, h = -1 }"),
                "positive",
            ),
            (one(&format!("{good},\n  {good}")), "twice"),
            (
                one(&format!(
                    "{good},\n  {{ hid = 0x3a, x = 0.5, y = 0.5, w = 1, h = 1 }}"
                )),
                "over key 0x29",
            ),
            (
                one("{ hid = 0x29, x = 0, y = 0, w = 1, h = 1, depth = 2 }"),
                "unknown field",
            ),
            (one("{ hid = 0x129, x = 0, y = 0, w = 1, h = 1 }"), ""),
            (
                format!(
                    "{}[logo]\nlid = 1.3\nx = 0.5\ny = 0.5\nw = 1\nh = 1\n",
                    one(good)
                ),
                "the logo: off the lid",
            ),
            (
                format!(
                    "{}[logo]\nlid = 0\nx = 0.1\ny = 0.1\nw = 0.1\nh = 0.1\n",
                    one(good)
                ),
                "the logo: lid",
            ),
            (
                format!(
                    "{}[touchpad]\nx = 0\ny = 2\nw = 4\nh = 2\nrows = 9\n",
                    one(good)
                ),
                "rows",
            ),
            ("name = \"x\"\nkeys = []\n".to_owned(), "no keys"),
            ("keys = [".to_owned(), ""),
        ] {
            let err = Layout::parse(&text).unwrap_err();
            assert!(err.contains(why), "{text}: {err}");
        }
        // Keys that only touch are fine.
        let touching = one(&format!(
            "{good},\n  {{ hid = 0x3a, x = 1, y = 0, w = 1, h = 1 }}"
        ));
        assert!(Layout::parse(&touching).is_ok());
    }

    #[test]
    fn looks_in_the_xdg_dirs() {
        let dirs = |home: Option<&str>, user: Option<&str>, system: Option<&str>| {
            dirs(
                user.map(Into::into),
                home.map(Into::into),
                None,
                system.map(Into::into),
            )
        };
        assert_eq!(
            dirs(Some("/home/me"), None, None),
            [
                "/home/me/.local/share/ryukaze/keyboards",
                "/usr/local/share/ryukaze/keyboards",
                "/usr/share/ryukaze/keyboards",
            ]
            .map(PathBuf::from)
        );
        assert_eq!(
            dirs(
                Some("/home/me"),
                Some("/data"),
                Some("/run/current-system/sw/share:relative:")
            ),
            [
                "/data/ryukaze/keyboards",
                "/run/current-system/sw/share/ryukaze/keyboards"
            ]
            .map(PathBuf::from)
        );
        assert_eq!(
            dirs(None, Some("relative"), Some("/a")),
            [PathBuf::from("/a/ryukaze/keyboards")]
        );
        // Beside the program, as Nix installs it, after the user's own.
        assert_eq!(
            super::dirs(
                None,
                Some("/home/me".into()),
                Some("/nix/store/abc-ryukaze/bin/ryukaze".into()),
                Some("/usr/share".into()),
            ),
            [
                "/home/me/.local/share/ryukaze/keyboards",
                "/nix/store/abc-ryukaze/share/ryukaze/keyboards",
                "/usr/share/ryukaze/keyboards",
            ]
            .map(PathBuf::from)
        );
    }

    fn write(dir: &Path, name: &str, layout: &str, boards: &str) {
        let text = format!(
            "name = \"{layout}\"\nboards = [{boards}]\nkeys = [\n  {{ hid = 0x29, label = \"Esc\", x = 0, y = 0, w = 1, h = 1 }},\n]\n"
        );
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(name), text).unwrap();
    }

    #[test]
    fn a_file_overrides_the_built_in_one() {
        let root = tempfile::tempdir().unwrap();
        let (user, system) = (root.path().join("user"), root.path().join("system"));
        let names =
            |dirs: &[PathBuf], board: Option<&str>| pick(load(dirs), board, None).unwrap().name;
        let both = [user.clone(), system.clone()];

        // Nothing there: the built-in ones, by board.
        assert_eq!(names(&both, Some("MS-1824")), "Titan 18 HX");
        assert_eq!(names(&both, Some("MS-17KK")), "Alpha 17 C7VG");
        assert_eq!(names(&both, None), "Alpha 17 C7VG");

        write(&system, "titan.toml", "system titan", "\"MS-1824\"");
        assert_eq!(names(&both, Some("MS-1824")), "system titan");
        assert_eq!(names(&both, Some("MS-17KK")), "Alpha 17 C7VG");
        // The user's before the system's, the default too.
        write(&user, "mine.toml", "user titan", "\"ms-1824\"");
        write(&user, "any.toml", "user default", "");
        assert_eq!(names(&both, Some("MS-1824")), "user titan");
        assert_eq!(names(&both, Some("MS-17KK")), "user default");
        assert_eq!(
            names(&[system.clone(), user.clone()], Some("MS-1824")),
            "system titan"
        );
    }

    #[test]
    fn a_broken_file_is_skipped() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("keyboards");
        write(&dir, "a.toml", "first", "\"MS-1824\"");
        fs::write(dir.join("0-broken.toml"), "name = \"broken\"\nboards = [\"MS-1824\"]\nkeys = [{ hid = 0x32, x = 0, y = 0, w = 1, h = 1 }]\n").unwrap();
        fs::write(dir.join("1-garbled.toml"), "name = ").unwrap();
        fs::write(dir.join("notes.txt"), "not a layout").unwrap();
        let layouts = load(&[dir]);
        let names: Vec<&str> = layouts.iter().map(|layout| layout.name.as_str()).collect();
        assert_eq!(names, ["first", "Titan 18 HX", "Alpha 17 C7VG"]);
        assert_eq!(pick(layouts, Some("MS-1824"), None).unwrap().name, "first");
    }

    #[test]
    fn a_file_for_the_laptops_chid() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("keyboards");
        let chid = layout::chid("Micro-Star International Co., Ltd.", "Alpha 17 C7VG");
        let text = format!(
            "name = \"by chid\"\nchids = [\"{}\"]\nkeys = [{{ hid = 0x29, x = 0, y = 0, w = 1, h = 1 }}]\n",
            chid.to_uppercase()
        );
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.toml"), text).unwrap();
        let name = |board, chid| {
            pick(load(std::slice::from_ref(&dir)), board, chid)
                .unwrap()
                .name
        };
        assert_eq!(name(Some("MS-17KK"), Some(chid.as_str())), "by chid");
        assert_eq!(name(Some("MS-1824"), Some(chid.as_str())), "by chid");
        // Not for any other laptop, as a file with no CHIDs and boards is.
        let other = layout::chid("Micro-Star International Co., Ltd.", "Alpha 17 C7VF");
        assert_eq!(name(Some("MS-17KK"), Some(other.as_str())), "Alpha 17 C7VG");
        assert_eq!(name(Some("MS-1824"), None), "Titan 18 HX");
    }
}
