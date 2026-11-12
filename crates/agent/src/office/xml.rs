use quick_xml::{events::Event, Reader, XmlVersion};
use std::ops::Range;

pub(super) struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub outer: Range<usize>,
    pub inner: Range<usize>,
}

impl Element {
    pub fn local(&self) -> &str {
        self.name.rsplit(':').next().unwrap()
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    pub fn local_attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(key, _)| key.rsplit(':').next() == Some(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn tag(&self, changes: &[(&str, &str)], empty: bool) -> String {
        let mut tag = format!("<{}", self.name);
        for (name, value) in &self.attrs {
            if !changes.iter().any(|(changed, _)| changed == name) {
                tag.push_str(&format!(" {name}=\"{}\"", escape(value)));
            }
        }
        for (name, value) in changes {
            tag.push_str(&format!(" {name}=\"{}\"", escape(value)));
        }
        tag.push_str(if empty { "/>" } else { ">" });
        tag
    }
}

pub(super) struct Xml<'a> {
    pub source: &'a str,
    pub elements: Vec<Element>,
}

impl<'a> Xml<'a> {
    pub fn parse(source: &'a str) -> Result<Self, String> {
        let mut reader = Reader::from_str(source);
        let mut elements: Vec<Element> = Vec::new();
        let mut stack: Vec<usize> = Vec::new();
        let mut roots = 0;
        loop {
            let start = reader.buffer_position() as usize;
            let event = reader.read_event().map_err(|error| error.to_string())?;
            let end = reader.buffer_position() as usize;
            match event {
                Event::Start(ref tag) | Event::Empty(ref tag) => {
                    if elements.len() >= 750_000 || stack.len() >= 96 {
                        return Err("Office XML exceeds its element or depth limit".into());
                    }
                    let index = elements.len();
                    let parent = stack.last().copied();
                    if let Some(parent) = parent {
                        elements[parent].children.push(index);
                    } else {
                        roots += 1;
                    }
                    let mut attrs = Vec::new();
                    for attr in tag.attributes() {
                        let attr = attr.map_err(|error| error.to_string())?;
                        let name = attr.key.as_ref();
                        let value = attr
                            .normalized_value(XmlVersion::Implicit1_0)
                            .map_err(|error| error.to_string())?;
                        attrs.push((name.to_owned(), value.into_owned()));
                    }
                    elements.push(Element {
                        name: tag.name().as_ref().to_owned(),
                        attrs,
                        parent,
                        outer: start..end,
                        inner: end..end,
                        children: Vec::new(),
                    });
                    if matches!(event, Event::Start(_)) {
                        stack.push(index);
                    }
                }
                Event::End(_) => {
                    let index = stack
                        .pop()
                        .ok_or("Office XML has an unmatched closing tag")?;
                    elements[index].inner.end = start;
                    elements[index].outer.end = end;
                }
                Event::DocType(_) => return Err("Office XML with a DTD is unsupported".into()),
                Event::GeneralRef(value) => {
                    let entity = format!("&{};", value.as_ref());
                    quick_xml::escape::unescape(&entity).map_err(|error| error.to_string())?;
                }
                Event::Text(value)
                    if stack.is_empty() && !value.xml10_content().trim().is_empty() =>
                {
                    return Err("Office XML contains text outside the root".into());
                }
                Event::Eof => break,
                _ => {}
            }
        }
        if !stack.is_empty() || roots != 1 {
            return Err("Office XML requires one complete root".into());
        }
        Ok(Self { source, elements })
    }

    pub fn named<'b>(&'b self, name: &'b str) -> impl Iterator<Item = usize> + 'b {
        self.elements
            .iter()
            .enumerate()
            .filter(move |(_, element)| element.local() == name)
            .map(|(index, _)| index)
    }

    pub fn children<'b>(&'b self, index: usize, name: &'b str) -> impl Iterator<Item = usize> + 'b {
        self.elements[index]
            .children
            .iter()
            .copied()
            .filter(move |&child| self.elements[child].local() == name)
    }

    pub fn child(&self, index: usize, name: &str) -> Option<usize> {
        self.children(index, name).next()
    }

    pub fn ancestors(&self, index: usize) -> impl Iterator<Item = usize> + '_ {
        std::iter::successors(self.elements[index].parent, |&node| {
            self.elements[node].parent
        })
    }

    pub fn descendants(&self, index: usize) -> impl Iterator<Item = usize> + '_ {
        let end = self.elements[index].inner.end;
        (index + 1..self.elements.len())
            .take_while(move |&child| self.elements[child].outer.start < end)
    }

    pub fn text(&self, index: usize) -> Result<String, String> {
        let node = &self.elements[index];
        if !node.children.is_empty() {
            return Err("Office text element contains nested XML".into());
        }
        quick_xml::escape::unescape(&self.source[node.inner.clone()])
            .map(|text| text.into_owned())
            .map_err(|error| error.to_string())
    }
}

pub(super) fn escape(value: &str) -> std::borrow::Cow<'_, str> {
    quick_xml::escape::escape(value)
}

pub(super) fn replace_ranges(xml: &str, mut changes: Vec<(Range<usize>, String)>) -> String {
    changes.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut result = xml.to_owned();
    for (range, value) in changes {
        result.replace_range(range, &value);
    }
    result
}

pub(super) fn validate_text(text: &str) -> Result<(), String> {
    if text.len() > 1024 * 1024
        || text.chars().any(|character| {
            (character.is_control() && !matches!(character, '\t' | '\r' | '\n'))
                || matches!(character, '\u{fffe}' | '\u{ffff}')
        })
    {
        return Err("Office text exceeds the limit or contains invalid XML characters".into());
    }
    Ok(())
}
