use quick_xml::events::Event;
use quick_xml::reader::Reader;

/// The handful of `ComicInfo.xml` fields the header actually shows — see
/// <https://anansi-project.github.io/docs/comicinfo/documentation> for the
/// full schema, most of which (writer, genre, page-level tags, ...) nothing
/// here reads. Every field is independently optional: taggers routinely fill
/// in only some of them, and a `ComicInfo.xml` with just a `<Series>` is
/// still worth showing that much.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ComicInfo {
    pub series: Option<String>,
    pub number: Option<String>,
    pub title: Option<String>,
}

impl ComicInfo {
    /// Whether there's anything here worth showing at all — an archive with
    /// no `ComicInfo.xml`, or one quick-xml couldn't make sense of a single
    /// recognized field from, still parses to a `Default`, so callers check
    /// this rather than just `is_some()` on the `Option` around it.
    pub fn is_empty(&self) -> bool {
        self.series.is_none() && self.number.is_none() && self.title.is_none()
    }

    /// Parses `ComicInfo.xml`'s raw bytes. Tolerant by design: malformed XML
    /// or an unrecognized element just leaves the corresponding field (or
    /// all of them) `None` rather than failing outright — a comic archive's
    /// pages are still perfectly readable even if its metadata is garbled.
    pub fn parse(bytes: &[u8]) -> Self {
        let mut reader = Reader::from_reader(bytes);

        let mut info = ComicInfo::default();
        let mut current: Option<Field> = None;
        // Accumulated across every `Text`/`GeneralRef` event between a
        // recognized field's `Start` and its `End` — quick-xml splits text
        // around each entity reference (e.g. `Foo &amp; Bar` arrives as
        // `Text("Foo")`, `GeneralRef("amp")`, `Text("Bar")`) rather than
        // handing back one already-unescaped string, so a field's full text
        // has to be reassembled from however many pieces it came in.
        let mut text = String::new();
        let mut buf = Vec::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(tag)) => {
                    current = Field::from_tag_name(tag.local_name().as_ref());
                    text.clear();
                }
                Ok(Event::Text(t)) => {
                    if current.is_some()
                        && let Ok(decoded) = t.decode()
                    {
                        text.push_str(&decoded);
                    }
                }
                Ok(Event::GeneralRef(r)) => {
                    if current.is_some()
                        && let Ok(name) = r.decode()
                    {
                        text.push_str(&resolve_entity(&name));
                    }
                }
                Ok(Event::End(_)) => {
                    if let Some(field) = current.take() {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            field.assign(&mut info, trimmed.to_string());
                        }
                    }
                    text.clear();
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
            buf.clear();
        }

        info
    }
}

/// Resolves one XML entity/character reference's name (the part between `&`
/// and `;`, e.g. `"amp"` or `"#39"`) to the text it stands for — the five
/// predefined XML entities plus decimal (`#NNN`) and hex (`#xNNNN`)
/// character references. An entity this doesn't recognize resolves to
/// nothing rather than failing the whole parse — that one character is
/// dropped from the field's text, everything else around it is kept.
fn resolve_entity(name: &str) -> String {
    if let Some(resolved) = quick_xml::escape::resolve_xml_entity(name) {
        return resolved.to_string();
    }
    let Some(digits) = name.strip_prefix('#') else { return String::new() };
    let code = match digits.strip_prefix(['x', 'X']) {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => digits.parse::<u32>().ok(),
    };
    code.and_then(char::from_u32).map(String::from).unwrap_or_default()
}

/// The `ComicInfo.xml` elements `parse` recognizes — everything else is
/// walked over and ignored.
#[derive(Clone, Copy)]
enum Field {
    Series,
    Number,
    Title,
}

impl Field {
    fn from_tag_name(tag: &[u8]) -> Option<Self> {
        match tag {
            b"Series" => Some(Field::Series),
            b"Number" => Some(Field::Number),
            b"Title" => Some(Field::Title),
            _ => None,
        }
    }

    fn assign(self, info: &mut ComicInfo, text: String) {
        match self {
            Field::Series => info.series = Some(text),
            Field::Number => info.number = Some(text),
            Field::Title => info.title = Some(text),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_series_and_number() {
        let xml = br#"<?xml version="1.0"?>
            <ComicInfo xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
                <Series>One Piece</Series>
                <Number>105</Number>
            </ComicInfo>"#;
        let info = ComicInfo::parse(xml);
        assert_eq!(info.series.as_deref(), Some("One Piece"));
        assert_eq!(info.number.as_deref(), Some("105"));
        assert_eq!(info.title, None);
    }

    #[test]
    fn unescapes_a_named_entity_mid_string() {
        let xml = br#"<ComicInfo><Series>Fullmetal &amp; Alchemist</Series></ComicInfo>"#;
        assert_eq!(ComicInfo::parse(xml).series.as_deref(), Some("Fullmetal & Alchemist"));
    }

    #[test]
    fn resolves_decimal_and_hex_character_references() {
        let xml = br#"<ComicInfo><Series>Caf&#233; &#x2014; vol.</Series></ComicInfo>"#;
        assert_eq!(ComicInfo::parse(xml).series.as_deref(), Some("Café — vol."));
    }

    #[test]
    fn ignores_unrecognized_fields() {
        let xml = br#"<ComicInfo><Writer>Eiichiro Oda</Writer><Series>One Piece</Series></ComicInfo>"#;
        let info = ComicInfo::parse(xml);
        assert_eq!(info.series.as_deref(), Some("One Piece"));
    }

    #[test]
    fn malformed_input_yields_an_empty_result_instead_of_panicking() {
        let info = ComicInfo::parse(b"this is not xml at all \x00\x01\x02");
        assert!(info.is_empty());
    }

    #[test]
    fn empty_bytes_yield_an_empty_result() {
        assert!(ComicInfo::parse(b"").is_empty());
    }
}
