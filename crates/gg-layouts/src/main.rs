//! ryukaze-gg-layouts: keyboard layout files for ryukaze's window, measured
//! from the pictures of a SteelSeries GG install of the user's own, and
//! the laptops GG takes each keyboard for (see docs/gg-layouts.md).

mod asar;
mod beside;
mod boxes;
mod css;
mod engine;
mod fit;
mod measure;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use anyhow::{Context, Result, bail};
use ryukaze_core::keyboard;

const USAGE: &str = "usage: ryukaze-gg-layouts [--device NAME]... [--label HID=TEXT]... (--out DIR | --file FILE) GG...
  GG: SteelSeries GG's install directory, its unpacked installer, or its app.asar
  --device NAME    only this GG device's layout, such as msi_titan18_klc
  --label HID=TEXT label the key of this HID usage so, such as 0xe4=Copilot
  --out DIR        a file a layout in DIR, by the device's name
with one --device:
  --file FILE      the layout to FILE
  --name NAME      named NAME rather than as GG names the device
  --board BOARD    for laptops of this DMI board name too
  --for-any        for any laptop: none by board or CHID";

/// How deep under a directory GG's files are looked for.
const DEPTH: usize = 8;

const ENGINE: &str = "SteelSeriesEngine.exe";

/// What to write, and where.
#[derive(Debug, Default)]
struct Options {
    out: PathBuf,
    /// `out` is the one layout's file rather than a directory.
    file: bool,
    only: BTreeSet<String>,
    labels: BTreeMap<u8, String>,
    name: Option<String>,
    boards: Vec<String>,
    for_any: bool,
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut options = Options::default();
    let (mut out, mut file, mut sources) = (None, None, Vec::new());
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" | "-o" => {
                out = Some(PathBuf::from(value(&mut args, "--out takes a directory")))
            }
            "--file" => file = Some(PathBuf::from(value(&mut args, "--file takes a file"))),
            "--device" => {
                options
                    .only
                    .insert(value(&mut args, "--device takes a name"));
            }
            "--label" => {
                let label = value(&mut args, "--label takes HID=TEXT");
                let (code, text) = parse_label(&label)
                    .unwrap_or_else(|| usage(&format!("--label {label:?}: not HID=TEXT")));
                options.labels.insert(code, text);
            }
            "--name" => options.name = Some(value(&mut args, "--name takes a name")),
            "--board" => options
                .boards
                .push(value(&mut args, "--board takes a DMI board name")),
            "--for-any" => options.for_any = true,
            "--help" | "-h" => {
                println!("{USAGE}");
                return;
            }
            _ if arg.starts_with('-') => usage(&format!("unknown argument {arg:?}")),
            _ => sources.push(PathBuf::from(arg)),
        }
    }
    let one =
        options.name.is_some() || !options.boards.is_empty() || options.for_any || file.is_some();
    if one && options.only.len() != 1 {
        usage("--file, --name, --board and --for-any go with one --device");
    }
    (options.out, options.file) = match (out, file) {
        (Some(dir), None) => (dir, false),
        (None, Some(file)) => (file, true),
        _ => usage("--out or --file, one of them"),
    };
    if sources.is_empty() {
        usage("no GG to read");
    }
    if let Err(err) = run(&sources, &options) {
        eprintln!("ryukaze-gg-layouts: {err:#}");
        std::process::exit(1);
    }
}

/// An option's value, the next argument, or else the usage and `what`.
fn value(args: &mut impl Iterator<Item = String>, what: &str) -> String {
    args.next().unwrap_or_else(|| usage(what))
}

/// `0xe4=Copilot` or `228=Copilot`.
fn parse_label(arg: &str) -> Option<(u8, String)> {
    let (code, text) = arg.split_once('=')?;
    let code = match code.strip_prefix("0x").or_else(|| code.strip_prefix("0X")) {
        Some(hex) => u8::from_str_radix(hex, 16).ok()?,
        None => code.parse().ok()?,
    };
    Some((code, text.to_owned()))
}

fn usage(problem: &str) -> ! {
    eprintln!("{problem}\n{USAGE}");
    std::process::exit(2);
}

/// GG's files found under the sources.
#[derive(Debug, Default)]
struct Found {
    archives: Vec<PathBuf>,
    engines: Vec<PathBuf>,
}

fn find(sources: &[PathBuf]) -> Result<Found> {
    let mut found = Found::default();
    for source in sources {
        let meta = fs::metadata(source).with_context(|| format!("{}", source.display()))?;
        if meta.is_dir() {
            walk(source, DEPTH, &mut found)?;
        } else if source
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case(ENGINE))
        {
            found.engines.push(source.clone());
        } else {
            found.archives.push(source.clone());
        }
    }
    Ok(found)
}

fn walk(dir: &Path, depth: usize, found: &mut Found) -> Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .with_context(|| format!("{}", dir.display()))?
        .collect::<Result<_, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let (path, kind) = (entry.path(), entry.file_type()?);
        let name = entry.file_name();
        if kind.is_dir() && depth > 0 && !name.to_string_lossy().ends_with(".asar.unpacked") {
            walk(&path, depth - 1, found)?;
        } else if kind.is_file() && name == "app.asar" {
            found.archives.push(path);
        } else if kind.is_file() && name.to_string_lossy().eq_ignore_ascii_case(ENGINE) {
            found.engines.push(path);
        }
    }
    Ok(())
}

/// A device's markers and pictures, and the archive and style sheet they
/// came from.
struct Source<'a> {
    archive: &'a asar::Archive,
    sheet: String,
    device: css::Device,
}

/// The CHIDs of each device's keyboard controller, from the engine.
fn chids(engines: &[PathBuf]) -> Result<BTreeMap<String, BTreeSet<String>>> {
    let Some(exe) = engines.first() else {
        bail!("no {ENGINE} found: the layouts will be for no laptop in particular");
    };
    let migrations = exe
        .parent()
        .map(|dir| dir.join("db/migrations"))
        .context("the engine in no directory")?;
    let devices = engine::Devices::read(&migrations)?;
    let image = engine::Image::parse(fs::read(exe).with_context(|| format!("{}", exe.display()))?)
        .with_context(|| format!("{}", exe.display()))?;
    let table =
        engine::Table::read(&image, &devices).with_context(|| format!("{}", exe.display()))?;
    eprintln!(
        "{}: {} CHIDs, {} for no device of the database's",
        exe.display(),
        table.entries.len(),
        table.unnamed
    );
    Ok(table.keyboards())
}

fn run(sources: &[PathBuf], options: &Options) -> Result<()> {
    let found = find(sources)?;
    if found.archives.is_empty() {
        bail!(
            "no app.asar found in {}",
            sources
                .iter()
                .map(|s| s.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let archives: Vec<asar::Archive> = found
        .archives
        .iter()
        .map(|path| asar::Archive::open(path))
        .collect::<Result<_>>()?;
    let mut devices: BTreeMap<String, Source> = BTreeMap::new();
    for archive in &archives {
        let sheets: Vec<String> = archive
            .names()
            .filter(|name| name.ends_with(".css"))
            .map(str::to_owned)
            .collect();
        for sheet in sheets {
            let text = archive.read(&sheet)?;
            for (name, device) in css::devices(&css::rules(&String::from_utf8_lossy(&text))) {
                devices.entry(name).or_insert_with(|| Source {
                    archive,
                    sheet: sheet.clone(),
                    device,
                });
            }
        }
    }
    let only = &options.only;
    if let Some(missing) = only.iter().find(|name| !devices.contains_key(*name)) {
        bail!("no device {missing:?} in GG's style sheets");
    }
    let chids = if options.for_any {
        BTreeMap::new()
    } else {
        chids(&found.engines).unwrap_or_else(|err| {
            eprintln!("ryukaze-gg-layouts: {err:#}");
            BTreeMap::new()
        })
    };

    let wanted: Vec<(&String, &Source)> = devices
        .iter()
        .filter(|(name, source)| {
            (only.is_empty() || only.contains(*name)) && !source.device.markers.is_empty()
        })
        .collect();
    if !options.file {
        fs::create_dir_all(&options.out).with_context(|| format!("{}", options.out.display()))?;
    }
    let (mut written, mut failed, mut others) = (0, 0, 0);
    for (name, measured) in measure_all(&wanted) {
        let measured = match measured {
            Ok(measured) => measured,
            Err(Skip::NotKeyboard) => {
                others += 1;
                continue;
            }
            Err(Skip::Failed(err)) => {
                eprintln!("{name}: {err:#}");
                failed += 1;
                continue;
            }
        };
        let chids: Vec<String> = chids.get(name).into_iter().flatten().cloned().collect();
        let made = measure::layout(name, chids, &measured.keys).map_err(anyhow::Error::msg);
        let (mut file, gap) = match made {
            Ok(made) => made,
            Err(err) => {
                eprintln!("{name}: {err:#}");
                failed += 1;
                continue;
            }
        };
        for key in &mut file.keys {
            if let Some(label) = options.labels.get(&key.hid) {
                key.label.clone_from(label);
            }
        }
        if let Some(name) = &options.name {
            file.name.clone_from(name);
        }
        file.boards.clone_from(&options.boards);
        let mut notes = Vec::new();
        if let Some(alc) = name
            .strip_suffix("_klc")
            .map(|prefix| format!("{prefix}_alc"))
            && let Some(source) = devices.get(&alc)
        {
            match beside::measure(
                &source.device,
                source.archive,
                &source.sheet,
                &file.keys,
                gap,
            ) {
                Ok(found) => {
                    let mut with = file.clone();
                    (with.touchpad, with.deck, with.logo) =
                        (found.touchpad, found.deck, found.logo);
                    notes.extend(found.notes.into_iter().map(|note| format!("{alc}: {note}")));
                    match with.check() {
                        Ok(()) => file = with,
                        Err(err) => notes.push(format!("{alc}: left out, {err}")),
                    }
                }
                Err(err) => notes.push(format!("{alc}: {err:#}")),
            }
        }
        let path = if options.file {
            options.out.clone()
        } else {
            options.out.join(format!("{name}.toml"))
        };
        fs::write(&path, measure::text(&file)).with_context(|| format!("{}", path.display()))?;
        let off: Vec<String> = measured
            .unplaced
            .iter()
            .map(|code| format!(" {code:#04x}"))
            .collect();
        let parts: Vec<&str> = [
            file.touchpad.is_some().then_some("touchpad"),
            file.deck.is_some().then_some("deck"),
            file.logo.is_some().then_some("logo"),
        ]
        .into_iter()
        .flatten()
        .collect();
        eprintln!(
            "{name}: {} keys from {}{}, {} CHIDs, markers on no key:{}",
            file.keys.len(),
            measured.picture,
            if parts.is_empty() {
                String::new()
            } else {
                format!(", its {}", parts.join(" and "))
            },
            file.chids.len(),
            if off.is_empty() {
                " none".to_owned()
            } else {
                off.concat()
            }
        );
        for note in notes {
            eprintln!("  {note}");
        }
        written += 1;
    }
    eprintln!(
        "{written} layouts in {}, {failed} keyboards failed, {others} other devices passed over",
        options.out.display()
    );
    if written == 0 {
        bail!("no layouts written");
    }
    Ok(())
}

/// Each device measured, a thread a core.
fn measure_all<'a>(devices: &[(&'a String, &Source)]) -> Vec<(&'a String, Result<Measured, Skip>)> {
    let next = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(1, usize::from);
    let mut measured: Vec<(usize, Result<Measured, Skip>)> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads.min(devices.len()))
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let at = next.fetch_add(1, Ordering::Relaxed);
                        let Some((name, source)) = devices.get(at) else {
                            return done;
                        };
                        done.push((at, measure_device(name, source)));
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().expect("a measuring thread panicked"))
            .collect()
    });
    measured.sort_by_key(|(at, _)| *at);
    measured
        .into_iter()
        .map(|(at, measured)| (devices[at].0, measured))
        .collect()
}

enum Skip {
    /// No Esc and letters among its markers.
    NotKeyboard,
    Failed(anyhow::Error),
}

/// The keys a device's best picture shows.
struct Measured {
    keys: BTreeMap<u8, boxes::Patch>,
    unplaced: Vec<u16>,
    picture: String,
}

fn measure_device(name: &str, source: &Source) -> Result<Measured, Skip> {
    let known: BTreeSet<u8> = keyboard::names()
        .into_iter()
        .map(|(_, code)| code)
        .collect();
    let markers: BTreeMap<u16, (f32, f32)> = source
        .device
        .markers
        .iter()
        .filter(|(code, _)| u8::try_from(**code).is_ok_and(|code| known.contains(&code)))
        .map(|(code, marker)| (*code, marker.centre()))
        .collect();
    let letters = markers
        .keys()
        .filter(|&&code| (0x04..=0x1d).contains(&code))
        .count();
    if !markers.contains_key(&0x29) || letters < 10 {
        return Err(Skip::NotKeyboard);
    }
    let mut best: Option<(fit::Fit, String)> = None;
    let mut problems = Vec::new();
    for picture in &source.device.pictures {
        let Some(path) = beside(&source.sheet, &picture.url) else {
            continue;
        };
        let decoded = source
            .archive
            .read(&path)
            .and_then(|bytes| boxes::decode(&bytes));
        let decoded = match decoded {
            Ok(decoded) => decoded,
            Err(err) => {
                problems.push(format!("{path}: {err:#}"));
                continue;
            }
        };
        for solid in boxes::SOLID {
            let keys = decoded.keys(solid);
            let drawn = picture.drawn(decoded.width, decoded.height);
            let Some(fit) = fit::fit(&markers, &keys, decoded.width, decoded.height, drawn) else {
                continue;
            };
            let what = match picture.region {
                Some(region) => format!("region {region}'s picture"),
                None => "its picture".to_owned(),
            };
            let better = best.as_ref().is_none_or(|(other, _)| {
                (fit.placed.len(), std::cmp::Reverse(fit.spare))
                    > (other.placed.len(), std::cmp::Reverse(other.spare))
            });
            if better {
                best = Some((fit, what));
            }
        }
    }
    let Some((fit, picture)) = best else {
        let why = if problems.is_empty() {
            "no picture of its keys".to_owned()
        } else {
            problems.join("; ")
        };
        return Err(Skip::Failed(anyhow::anyhow!("{why}")));
    };
    if fit.placed.len() * 2 < markers.len() {
        return Err(Skip::Failed(anyhow::anyhow!(
            "only {} of {} markers on keys of {name}'s pictures",
            fit.placed.len(),
            markers.len()
        )));
    }
    let keys = fit
        .placed
        .iter()
        .map(|(&code, &patch)| (code as u8, patch))
        .collect();
    Ok(Measured {
        keys,
        unplaced: fit.unplaced,
        picture,
    })
}

/// The archive path of `url` as the style sheet `sheet` names it.
fn beside(sheet: &str, url: &str) -> Option<String> {
    if url.contains(':') || url.starts_with("//") {
        return None;
    }
    let url = url.split(['?', '#']).next()?;
    let mut parts: Vec<&str> = if url.starts_with('/') {
        Vec::new()
    } else {
        sheet.split('/').collect()
    };
    if !url.starts_with('/') {
        parts.pop();
    }
    for part in url.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    Some(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use ryukaze_core::layout::{self, File, Logo};

    use super::*;

    const MSI: &str = "Micro-Star International Co., Ltd.";

    fn to(out: &Path, devices: &[&str]) -> Options {
        Options {
            out: out.to_owned(),
            only: devices.iter().map(|device| device.to_string()).collect(),
            ..Options::default()
        }
    }

    /// A board as GG might draw it: (HID usage, column, row, width), in
    /// keys.
    fn board() -> Vec<(u16, f32, f32, f32)> {
        let mut keys = vec![(0x29, 0.0, 0.0, 1.0)];
        let mut row = |row: f32, from: f32, codes: &[u16], ends: (f32, f32), last: u16| {
            keys.push((codes[0], 0.0, row, ends.0));
            for (at, &code) in codes[1..].iter().enumerate() {
                keys.push((code, from + at as f32, row, 1.0));
            }
            keys.push((last, from + (codes.len() - 1) as f32, row, ends.1));
        };
        row(
            1.0,
            1.0,
            &[
                0x35, 0x1e, 0x1f, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x2d, 0x2e,
            ],
            (1.0, 2.0),
            0x2a,
        );
        row(
            2.0,
            1.5,
            &[
                0x2b, 0x14, 0x1a, 0x08, 0x15, 0x17, 0x1c, 0x18, 0x0c, 0x12, 0x13, 0x2f, 0x30,
            ],
            (1.5, 1.5),
            0x31,
        );
        row(
            3.0,
            1.75,
            &[
                0x39, 0x04, 0x16, 0x07, 0x09, 0x0a, 0x0b, 0x0d, 0x0e, 0x0f, 0x33, 0x34,
            ],
            (1.75, 2.25),
            0x28,
        );
        row(
            4.0,
            2.25,
            &[
                0xe1, 0x1d, 0x1b, 0x06, 0x19, 0x05, 0x11, 0x10, 0x36, 0x37, 0x38,
            ],
            (2.25, 2.75),
            0xe5,
        );
        keys
    }

    /// The board drawn `shift` keys to the right of where its markers say,
    /// 50 px a key, the keys 44 px.
    fn picture(shift: f32) -> Vec<u8> {
        let keys: Vec<boxes::Patch> = board()
            .iter()
            .map(|&(_, column, row, width)| boxes::Patch {
                x0: (30.0 + (column + shift) * 50.0) as u32,
                y0: (20.0 + row * 50.0) as u32,
                x1: (30.0 + (column + shift + width) * 50.0 - 6.0) as u32,
                y1: (20.0 + row * 50.0 + 44.0) as u32,
            })
            .collect();
        boxes::encode(&boxes::draw(810, 290, &keys))
    }

    /// The style sheet: the pictures drawn over 324 by 116 CSS pixels, the
    /// markers 16 px, 12 px of the picture short of where they show.
    fn sheet() -> String {
        let mut css = String::from(
            "#GameIntegration .device-test_klc .marker { width: 16px; height: 16px }\n\
             #GameIntegration .device-test_klc .deviceImg.region-1 .deviceImgContainer { background: url(img/jp.png); background-size: 324px 116px }\n\
             #GameIntegration .device-test_klc .deviceImg.region-3 .deviceImgContainer { background: url(./img/us.png); background-size: 324px 116px }\n\
             #GameIntegration .device-mouse .deviceImg { background-image: url(img/us.png) }\n\
             #GameIntegration .device-mouse .code1 { left: 1px; top: 1px }\n",
        );
        for (code, column, row, width) in board() {
            let (x, y) = (
                (30.0 + (column + width / 2.0) * 50.0 - 3.0 - 12.0) / 2.5,
                (20.0 + row * 50.0 + 22.0 - 12.0) / 2.5,
            );
            css.push_str(&format!(
                "#GameIntegration .device-test_klc .code{code} {{ left: {}px; top: {}px }}\n",
                x - 8.0,
                y - 8.0
            ));
        }
        css
    }

    fn chid_key(n: u32) -> String {
        format!("{n:08x}-1111-5222-8333-444455556666-KLC")
    }

    /// The light bar's picture: the lid 390 px wide, the deck's keys dark,
    /// 30 px a key from (430, 20), the touchpad's rows clear at (520, 200).
    fn lights() -> Vec<u8> {
        let (width, height) = (1000, 300);
        let mut pixels = vec![[0u8; 4]; width * height];
        for (at, pixel) in pixels.iter_mut().enumerate() {
            let (x, y) = (at % width, at / width);
            if x < 390 {
                *pixel = [90, 90, 90, 255];
            } else if x >= 410 && !((520..700).contains(&x) && (200..260).contains(&y)) {
                *pixel = [120, 120, 120, 255];
            }
        }
        for (_, column, row, width_keys) in board() {
            let (x0, y0) = (430.0 + column * 30.0, 20.0 + row * 30.0);
            for y in y0 as usize..y0 as usize + 26 {
                for x in x0 as usize..(x0 + width_keys * 30.0) as usize - 4 {
                    pixels[y * width + x] = [50, 50, 50, 255];
                }
            }
        }
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(pixels.as_flattened()).unwrap();
        writer.finish().unwrap();
        bytes
    }

    /// Its zones, in CSS pixels, half the picture's: the touchpad's rows,
    /// and the logo at 300 to 340 across and 100 to 150 down.
    const LIGHTS: &str = "
#GameIntegration .device-test_alc .zoneBackgrounds #ZoneColors .code0 { left: 260px; top: 100px; width: 90px; height: 10px }
#GameIntegration .device-test_alc .zoneBackgrounds #ZoneColors .code1 { left: 260px; top: 110px; width: 90px; height: 10px }
#GameIntegration .device-test_alc .zoneBackgrounds #ZoneColors .code2 { left: 260px; top: 120px; width: 90px; height: 10px }
#GameIntegration .device-test_alc .zoneBackgrounds #ZoneColors .code5 { left: 150px; top: 50px; width: 20px; height: 25px }
#GameIntegration .device-test_alc .deviceImg .deviceImgContainer { background: url(img/alc.png); width: 500px; height: 300px; background-size: contain }
";

    /// A GG install: its app.asar, and its engine with a table of 40 CHIDs,
    /// one of them a laptop with the test keyboard.
    fn install(root: &Path) {
        let us = picture(0.0);
        let jp = picture(0.5);
        let alc = lights();
        let sheet = sheet() + LIGHTS;
        let files: [(&str, &[u8]); 4] = [
            ("render/index.css", sheet.as_bytes()),
            ("render/img/us.png", &us),
            ("render/img/jp.png", &jp),
            ("render/img/alc.png", &alc),
        ];
        fs::create_dir_all(root.join("resources")).unwrap();
        fs::write(root.join("resources/app.asar"), asar::build(&files)).unwrap();

        let engine = root.join("apps/engine");
        fs::create_dir_all(engine.join("db/migrations/1.0.0")).unwrap();
        fs::write(
            engine.join("db/migrations/1.0.0/a.sql"),
            "INSERT INTO devices(id, product_id, name) VALUES (1, 272113752, 'test_klc');\n\
             INSERT INTO physical_devices(product_id, bootloader_product_id, name) VALUES (272113752, 272113753, 'test_klc');\n\
             INSERT INTO devices(id, product_id, name) VALUES (2, 272113664, 'other_klc');\n",
        )
        .unwrap();
        let mut keys: Vec<String> = (0..39).map(chid_key).collect();
        keys.insert(7, format!("{}-KLC", layout::chid(MSI, "Test 15 A1")));
        let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
        let values: Vec<u32> = (0..40)
            .map(|n| if n == 7 { 0x2059_2058 } else { 0x2001_2000 })
            .collect();
        fs::write(engine.join(ENGINE), engine::build(&keys, &values)).unwrap();
    }

    #[test]
    fn writes_a_layout_from_an_install() {
        let root = tempfile::tempdir().unwrap();
        install(&root.path().join("gg"));
        let out = root.path().join("out");
        run(&[root.path().join("gg")], &to(&out, &[])).unwrap();
        let written: Vec<_> = fs::read_dir(&out)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(written, ["test_klc.toml"]);
        let file = File::parse(&fs::read_to_string(out.join("test_klc.toml")).unwrap()).unwrap();
        assert_eq!(file.name, "test_klc");
        assert_eq!(file.chids, [layout::chid(MSI, "Test 15 A1")]);
        assert!(file.fits(None, Some(&layout::chid(MSI, " Test 15 A1 "))));
        assert_eq!(file.keys.len(), board().len());
        let near = |a: f32, b: f32| (a - b).abs() < 0.05;
        let pad = file.touchpad.unwrap();
        assert!(
            near(pad.x, 3.0) && near(pad.y, 6.0) && near(pad.w, 6.0) && near(pad.h, 2.0),
            "{pad:?}"
        );
        assert_eq!(pad.rows, 3);
        // The logo seen from the keys, the lid's other way round.
        assert_eq!(
            file.logo,
            Some(Logo {
                lid: 1.3,
                x: 0.128,
                y: 0.333,
                w: 0.103,
                h: 0.167
            })
        );
        for (code, column, row, width) in board() {
            let key = file
                .keys
                .iter()
                .find(|key| u16::from(key.hid) == code)
                .unwrap();
            let near = |a: f32, b: f32| (a - b).abs() < 0.03;
            assert!(
                near(key.x, column) && near(key.y, row) && near(key.w, width) && near(key.h, 1.0),
                "{key:?}"
            );
        }
    }

    #[test]
    fn takes_the_devices_asked_for() {
        let root = tempfile::tempdir().unwrap();
        install(root.path());
        let out = root.path().join("out");
        let asar = root.path().join("resources/app.asar");
        let err = run(std::slice::from_ref(&asar), &to(&out, &["nothing_klc"])).unwrap_err();
        assert!(
            err.to_string().contains("no device \"nothing_klc\""),
            "{err}"
        );
        run(std::slice::from_ref(&asar), &to(&out, &["test_klc"])).unwrap();
        // No engine given: for no laptop in particular.
        let file = File::parse(&fs::read_to_string(out.join("test_klc.toml")).unwrap()).unwrap();
        assert!(file.chids.is_empty() && file.for_any());
    }

    #[test]
    fn finds_pictures_beside_the_sheet() {
        assert_eq!(
            beside("render/index.css", "a.png").as_deref(),
            Some("render/a.png")
        );
        assert_eq!(
            beside("render/css/x.css", "../img/a.png?v=2").as_deref(),
            Some("render/img/a.png")
        );
        assert_eq!(
            beside("render/index.css", "/top.png").as_deref(),
            Some("top.png")
        );
        assert_eq!(beside("index.css", "../a.png"), None);
        assert_eq!(beside("index.css", "data:image/png;base64,AAAA"), None);
    }

    /// The built-in layouts made again from a GG copy of one's own, as
    /// docs/gg-layouts.md makes them: `RYUKAZE_GG=<GG> cargo test --release
    /// -p ryukaze-gg-layouts -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs a copy of GG, in RYUKAZE_GG"]
    fn makes_the_built_in_layouts_again() {
        let gg = PathBuf::from(std::env::var_os("RYUKAZE_GG").expect("RYUKAZE_GG"));
        let out = tempfile::tempdir().unwrap();
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/keyboards");
        let copilot = BTreeMap::from([(0xe4, "Copilot".to_owned())]);
        let built_in = [
            (
                "ms-1824.toml",
                Options {
                    name: Some("Titan 18 HX".into()),
                    boards: vec!["MS-1824".into()],
                    ..to(
                        &out.path().join("ms-1824.toml"),
                        &["msi_titan18_dragon_edition_klc"],
                    )
                },
            ),
            (
                "default.toml",
                Options {
                    name: Some("Alpha 17 C7VG".into()),
                    for_any: true,
                    ..to(&out.path().join("default.toml"), &["msi_gp76_klc"])
                },
            ),
        ];
        for (file, options) in built_in {
            let options = Options {
                file: true,
                labels: copilot.clone(),
                ..options
            };
            run(std::slice::from_ref(&gg), &options).unwrap();
            let made = fs::read_to_string(&options.out).unwrap();
            let known = fs::read_to_string(data.join(file)).unwrap();
            let (made, known) = (File::parse(&made).unwrap(), File::parse(&known).unwrap());
            let mut worst = [0.0f32; 4];
            for key in &known.keys {
                let other = made.keys.iter().find(|other| other.hid == key.hid).unwrap();
                let off = [
                    other.x - key.x,
                    other.y - key.y,
                    other.w - key.w,
                    other.h - key.h,
                ];
                for (worst, off) in worst.iter_mut().zip(off) {
                    *worst = worst.max(off.abs());
                }
            }
            println!(
                "{file}: worst x {:.3} y {:.3} w {:.3} h {:.3}",
                worst[0], worst[1], worst[2], worst[3]
            );
            assert_eq!(made, known, "{file}");
        }
    }
}
