use eframe::egui;
use std::{
    borrow::Cow,
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        LazyLock,
    },
};
use unicode_bidi::{BidiInfo, Level};
#[cfg(windows)]
use std::ptr::null_mut;
#[cfg(windows)]
use windows_sys::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};

const KEYS: &str = include_str!("../assets/translations/keys.txt");
const EXTRA: &str = include_str!("../assets/translations/extra.tsv");
const GROUPS: [(&str, usize); 4] = [
    (include_str!("../assets/translations/europe.tsv"), 3),
    (include_str!("../assets/translations/east-asia.tsv"), 4),
    (include_str!("../assets/translations/mideast.tsv"), 4),
    (include_str!("../assets/translations/south-asia.tsv"), 4),
];
pub const LANGUAGES: [(&str, &str); 16] = [
    ("pt", "Português (Brasil)"),
    ("en", "English"),
    ("es", "Español"),
    ("fr", "Français"),
    ("zh", "Chinese (Mandarin)"),
    ("ja", "Japanese"),
    ("ru", "Русский"),
    ("de", "Deutsch"),
    ("ar", "Arabic"),
    ("arz", "Egyptian Arabic"),
    ("id", "Bahasa Indonesia"),
    ("pcm", "Naija Pidgin"),
    ("hi", "Hindi"),
    ("bn", "Bengali"),
    ("ur", "Urdu"),
    ("mr", "Marathi"),
];
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static CATALOG: LazyLock<HashMap<&'static str, [&'static str; LANGUAGES.len()]>> =
    LazyLock::new(|| {
        let keys: Vec<_> = KEYS.lines().collect();
        let mut rows = vec![[""; LANGUAGES.len()]; keys.len()];
        let mut offset = 1;
        for (group, width) in GROUPS {
            for row in group.lines() {
                let mut parts = row.split('\t');
                let Some(index) = parts
                    .next()
                    .and_then(|id| id.parse::<usize>().ok())
                    .and_then(|id| id.checked_sub(1))
                else {
                    continue;
                };
                if let Some(cells) = rows.get_mut(index) {
                    for (cell, translated) in cells[offset..offset + width].iter_mut().zip(parts) {
                        *cell = translated;
                    }
                }
            }
            offset += width;
        }
        let mut catalog: HashMap<_, _> = keys.into_iter().zip(rows).collect();
        for row in EXTRA.lines() {
            let mut cells = row.split('\t');
            if let Some(key) = cells.next() {
                let mut translations = [""; LANGUAGES.len()];
                for (cell, translated) in translations[1..].iter_mut().zip(cells) {
                    *cell = translated;
                }
                catalog.insert(key, translations);
            }
        }
        catalog
    });
static TEMPLATES: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    let mut templates: Vec<_> = CATALOG
        .keys()
        .copied()
        .filter(|key| key.contains("{}"))
        .collect();
    templates.sort_unstable_by_key(|key| std::cmp::Reverse(key.len()));
    templates
});
type ShapedCatalog = HashMap<(&'static str, usize), String>;
static SHAPED: LazyLock<ShapedCatalog> = LazyLock::new(|| {
    let reshaper = arabic_reshaper::ArabicReshaper::new();
    let mut result = HashMap::new();
    for (&key, cells) in CATALOG.iter() {
        for language in [8, 9, 14] {
            let shaped = reshaper.reshape(cells[language]);
            let bidi = BidiInfo::new(&shaped, Some(Level::rtl()));
            if let Some(para) = bidi.paragraphs.first() {
                result.insert(
                    (key, language),
                    bidi.reorder_line(para, para.range.clone()).into_owned(),
                );
            }
        }
    }
    result
});

pub fn native(text: &str) -> &str {
    let language = ACTIVE.load(Ordering::Relaxed);
    translation(text, language)
}

fn translation(text: &str, language: usize) -> &str {
    if language == 0 {
        return match text {
            "Leiaute 2" => "Layout 2",
            "Leiaute 3" => "Layout 3",
            "Editar leiaute" => "Editar layout",
            "Editor de leiaute" => "Editor de layout",
            "Salvar leiaute" => "Salvar layout",
            "Leiaute" => "Layout",
            "Possível leiaute" => "Possível layout",
            "Leiaute personalizado salvo." => "Layout personalizado salvo.",
            "Ocultar leiaute padrão" => "Ocultar layout padrão",
            "Mostrar leiaute padrão" => "Mostrar layout padrão",
            "Leiaute selecionado." => "Layout selecionado.",
            "Leiaute incompatível com as teclas do perfil." => {
                "Layout incompatível com as teclas do perfil."
            }
            "Uma combinação usa um controle indisponível neste leiaute." => {
                "Uma combinação usa um controle indisponível neste layout."
            }
            "Chinese (Mandarin)" => "Chinês (mandarim)",
            "Japanese" => "Japonês",
            "Arabic" => "Árabe",
            "Egyptian Arabic" => "Árabe egípcio",
            "Bengali" => "Bengali",
            "Urdu" => "Urdu",
            "Marathi" => "Marata",
            _ => text,
        };
    }
    CATALOG.get(catalog_key(text)).map_or(text, |cells| {
        let translated = cells[language];
        if translated.is_empty() {
            text
        } else {
            translated
        }
    })
}

fn catalog_key(text: &str) -> &str {
    match text {
        "Layout 2" => "Leiaute 2",
        "Layout 3" => "Leiaute 3",
        "Editar layout" => "Editar leiaute",
        "Editor de layout" => "Editor de leiaute",
        "Salvar layout" => "Salvar leiaute",
        "Layout" => "Leiaute",
        "Possível layout" => "Possível leiaute",
        "Layout personalizado salvo." => "Leiaute personalizado salvo.",
        "Ocultar layout padrão" => "Ocultar leiaute padrão",
        "Mostrar layout padrão" => "Mostrar leiaute padrão",
        "Leiaute selecionado." => "Layout selecionado.",
        "Leiaute incompatível com as teclas do perfil." => {
            "Layout incompatível com as teclas do perfil."
        }
        "Uma combinação usa um controle indisponível neste leiaute." => {
            "Uma combinação usa um controle indisponível neste layout."
        }
        _ => text,
    }
}

pub fn tr(text: &str) -> &str {
    let language = ACTIVE.load(Ordering::Relaxed);
    if matches!(language, 8 | 9 | 14) {
        if let Some(shaped) = SHAPED
            .get(&(catalog_key(text), language))
            .filter(|value| !value.is_empty())
        {
            return shaped;
        }
    }
    translation(text, language)
}

#[cfg(not(windows))]
pub fn tr_native(text: &str) -> &str {
    // AppKit and StatusNotifier menu hosts process bidi and Arabic shaping themselves.
    translation(text, ACTIVE.load(Ordering::Relaxed))
}

pub fn model(name: &str) -> &str {
    match name {
        "3 teclas e 1 botão giratório" => tr("3 teclas + 1 giratório"),
        _ => tr(name),
    }
}

fn capture_template<'a>(template: &str, text: &'a str) -> Option<Vec<&'a str>> {
    let mut parts = template.split("{}").peekable();
    let mut remaining = text.strip_prefix(parts.next()?)?;
    let mut captured = Vec::new();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            captured.push(remaining.strip_suffix(part)?);
            return Some(captured);
        }
        let (value, rest) = remaining.split_once(part)?;
        captured.push(value);
        remaining = rest;
    }
    None
}

fn fill_template(template: &str, args: &[&str]) -> String {
    let mut parts = template.split("{}");
    let mut result = parts.next().unwrap_or_default().to_owned();
    for (part, value) in parts.zip(args) {
        result.push_str(value);
        result.push_str(part);
    }
    result
}

fn logical_status(text: &str, language: usize) -> Cow<'_, str> {
    if language == 0 || CATALOG.contains_key(catalog_key(text)) {
        return Cow::Borrowed(translation(text, language));
    }
    if let Some(error) = text.strip_prefix("Erro: ") {
        return Cow::Owned(format!(
            "{}: {}",
            translation("Erro", language),
            logical_status(error, language)
        ));
    }
    for template in TEMPLATES.iter().copied() {
        if let Some(args) = capture_template(template, text) {
            let details: Vec<_> = args
                .iter()
                .map(|arg| logical_status(arg, language))
                .collect();
            let localized_args: Vec<_> = details.iter().map(AsRef::as_ref).collect();
            return Cow::Owned(fill_template(
                translation(template, language),
                &localized_args,
            ));
        }
    }
    // Shutdown status combines two catalog sentences or a sentence and a detailed error.
    if let Some((first, rest)) = text.split_once(". ") {
        let sentence = format!("{first}.");
        if CATALOG.contains_key(catalog_key(&sentence)) {
            return Cow::Owned(format!(
                "{} {}",
                translation(&sentence, language),
                logical_status(rest, language)
            ));
        }
    }
    Cow::Borrowed(text)
}

pub fn status(text: &str) -> String {
    let language = ACTIVE.load(Ordering::Relaxed);
    if CATALOG.contains_key(catalog_key(text)) {
        return tr(text).to_owned();
    }
    let logical = logical_status(text, language);
    if matches!(language, 8 | 9 | 14) && logical.as_ref() != text {
        let shaped = arabic_reshaper::ArabicReshaper::new().reshape(&logical);
        let bidi = BidiInfo::new(&shaped, Some(Level::rtl()));
        if let Some(para) = bidi.paragraphs.first() {
            return bidi.reorder_line(para, para.range.clone()).into_owned();
        }
    }
    logical.into_owned()
}

pub fn formatted(template: &str, args: &[&str]) -> String {
    status(&fill_template(template, args))
}

fn locale_index(locale: &str) -> Option<usize> {
    let lower = locale.trim().split(['.', '@']).next()?.replace('_', "-").to_ascii_lowercase();
    if lower == "ar-eg" || lower == "arz" {
        return Some(9);
    }
    let prefix = lower.split('-').next()?;
    LANGUAGES.iter().position(|(code, _)| *code == prefix)
}

#[cfg(windows)]
fn system_language() -> usize {
    let (mut count, mut length) = (0, 0);
    unsafe { GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, null_mut(), &mut length) };
    if length == 0 {
        return 0;
    }
    let mut names = vec![0; length as usize];
    if unsafe {
        GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut count,
            names.as_mut_ptr(),
            &mut length,
        )
    } == 0
    {
        return 0;
    }
    names
        .split(|value| *value == 0)
        .filter(|name| !name.is_empty())
        .find_map(|name| locale_index(&String::from_utf16_lossy(name)))
        .unwrap_or(0)
}

#[cfg(target_os = "linux")]
fn system_language() -> usize {
    ["LC_ALL", "LC_MESSAGES", "LANG"].into_iter()
        .filter_map(|key| std::env::var(key).ok())
        .find_map(|locale| locale_index(&locale))
        .unwrap_or(0)
}

#[cfg(target_os = "macos")]
fn system_language() -> usize {
    use std::ffi::{c_void, CStr};
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFLocaleCopyPreferredLanguages() -> *const c_void;
        fn CFArrayGetCount(array: *const c_void) -> isize;
        fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
        fn CFStringGetCString(text: *const c_void, buffer: *mut std::ffi::c_char, size: isize, encoding: u32) -> u8;
        fn CFRelease(value: *const c_void);
    }
    unsafe {
        let languages = CFLocaleCopyPreferredLanguages();
        if languages.is_null() { return 0; }
        let found = (0..CFArrayGetCount(languages)).find_map(|index| {
            let value = CFArrayGetValueAtIndex(languages, index);
            let mut buffer = [0; 96];
            (CFStringGetCString(value, buffer.as_mut_ptr(), buffer.len() as isize, 0x08000100) != 0)
                .then(|| CStr::from_ptr(buffer.as_ptr()).to_str().ok().and_then(locale_index))
                .flatten()
        });
        CFRelease(languages);
        found.unwrap_or(0)
    }
}

fn settings_file() -> PathBuf {
    crate::device::settings_dir().join("idioma.txt")
}

pub fn load(ctx: &egui::Context) -> Option<usize> {
    let setting = fs::read_to_string(settings_file())
        .ok()
        .and_then(|value| locale_index(&value));
    activate(ctx, setting);
    setting
}

pub fn save(ctx: &egui::Context, setting: Option<usize>) -> Result<(), String> {
    let file = settings_file();
    let temp = file.with_extension("novo");
    if let Some(parent) = file.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let content = setting.map_or("windows", |index| LANGUAGES[index].0);
    fs::write(&temp, content)
        .and_then(|()| fs::rename(temp, file))
        .map_err(|e| e.to_string())?;
    activate(ctx, setting);
    Ok(())
}

fn activate(ctx: &egui::Context, setting: Option<usize>) {
    let index = setting.unwrap_or_else(system_language);
    ACTIVE.store(index, Ordering::Relaxed);
    #[cfg(windows)]
    let files: &[&str] = match LANGUAGES[index].0 {
        "zh" => &["msyh.ttc", "simsun.ttc"],
        "ja" => &["YuGothR.ttc", "meiryo.ttc"],
        "hi" | "bn" | "mr" => &["Nirmala.ttc", "Nirmala.ttf", "mangal.ttf", "Vrinda.ttf"],
        "ar" | "arz" | "ur" => &["tahoma.ttf", "arial.ttf"],
        _ => &[],
    };
    #[cfg(windows)]
    let fallback = std::env::var_os("WINDIR")
        .map(|windows| PathBuf::from(windows).join("Fonts"))
        .and_then(|dir| files.iter().find_map(|file| fs::read(dir.join(file)).ok()));
    #[cfg(target_os = "linux")]
    let files: &[&str] = match LANGUAGES[index].0 {
        "zh" | "ja" => &["/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"],
        "hi" | "mr" => &["/usr/share/fonts/truetype/noto/NotoSansDevanagari-Regular.ttf"],
        "bn" => &["/usr/share/fonts/truetype/noto/NotoSansBengali-Regular.ttf"],
        "ar" | "arz" | "ur" => &["/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", "/usr/share/fonts/truetype/noto/NotoSansArabic-Regular.ttf"],
        _ => &[],
    };
    #[cfg(target_os = "macos")]
    let files: &[&str] = match LANGUAGES[index].0 {
        "zh" => &["/System/Library/Fonts/PingFang.ttc", "/System/Library/Fonts/Hiragino Sans GB.ttc"],
        "ja" => &["/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc", "/System/Library/Fonts/Hiragino Sans W3.ttc"],
        "hi" | "mr" => &["/System/Library/Fonts/Devanagari Sangam MN.ttc", "/System/Library/Fonts/Supplemental/Devanagari Sangam MN.ttc", "/System/Library/Fonts/Supplemental/DevanagariMT.ttc"],
        "bn" => &["/System/Library/Fonts/Bangla Sangam MN.ttc", "/System/Library/Fonts/Supplemental/Bangla Sangam MN.ttc"],
        "ar" | "arz" | "ur" => &["/System/Library/Fonts/Supplemental/Arial.ttf", "/System/Library/Fonts/GeezaPro.ttc"],
        _ => &[],
    };
    #[cfg(not(windows))]
    let fallback = files.iter().find_map(|file| fs::read(file).ok());
    let mut fonts = egui::FontDefinitions::default();
    if let Some(data) = fallback {
            let name = "windows-fallback".to_owned();
            fonts
                .font_data
                .insert(name.clone(), egui::FontData::from_owned(data).into());
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.entry(family).or_default().push(name.clone());
            }
    }
    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_fifteen_catalogs_translate_every_key() {
        assert_eq!(CATALOG.len(), KEYS.lines().count() + EXTRA.lines().count());
        assert!(EXTRA
            .lines()
            .all(|line| line.split('\t').count() == LANGUAGES.len()));
        for (key, cells) in CATALOG.iter() {
            for (language, value) in cells.iter().enumerate().skip(1) {
                assert!(
                    !value.is_empty(),
                    "{key} has no {} translation",
                    LANGUAGES[language].0
                );
                assert_eq!(
                    value.matches("{}").count(),
                    key.matches("{}").count(),
                    "Placeholder mismatch in {key} for {}",
                    LANGUAGES[language].0
                );
            }
        }
    }

    #[test]
    fn pt_displays_layout_and_other_languages_reuse_existing_translations() {
        assert_eq!(translation("Editor de leiaute", 0), "Editor de layout");
        assert_eq!(translation("Editor de layout", 1), "Layout editor");
        assert_eq!(translation("Layout excluído.", 1), "Layout deleted.");
    }

    #[test]
    fn formatted_status_keeps_os_details_and_named_controls() {
        assert_eq!(
            logical_status("Erro: Falha ao consultar USB HID. code 5", 1),
            "Error: Could not query USB HID. code 5"
        );
        assert_eq!(
            logical_status(
                "Salve primeiro as ações individuais de Key 2 na aba Teclado.",
                1
            ),
            "Save the individual actions for Key 2 in the Keyboard tab first."
        );
        assert_eq!(
            logical_status(
                "Teste inicial concluído. O próximo gesto pode desligar o computador.",
                1
            ),
            "Initial test completed. The next gesture can shut down the computer."
        );
    }
    #[test]
    fn windows_language_tags_fall_back_by_language_and_distinguish_arabic_variants() {
        assert_eq!(locale_index("pt-BR"), Some(0));
        assert_eq!(locale_index("fr-CA"), Some(3));
        assert_eq!(locale_index("zh-Hans"), Some(4));
        assert_eq!(locale_index("ar-EG"), Some(9));
        assert_eq!(locale_index("ar-SA"), Some(8));
        assert_eq!(locale_index("fi-FI"), None);
    }
}
