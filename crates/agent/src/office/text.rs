use super::xml::{escape, replace_ranges, validate_text, Xml};
use super::{Change, Item, Package, Section};

pub(super) fn word_sections(package: &Package, main: &str) -> Result<Vec<Section>, String> {
    let mut sections = vec![Section {
        id: main.to_owned(),
        label: "正文".to_owned(),
        kind: "document",
    }];
    for relation in package.relationships(main)? {
        let label = match relation.kind.as_str() {
            "header" => "页眉",
            "footer" => "页脚",
            _ => continue,
        };
        if !relation.external && !sections.iter().any(|section| section.id == relation.target) {
            sections.push(Section {
                label: format!("{label} {}", relation.id),
                id: relation.target,
                kind: "document",
            });
        }
    }
    Ok(sections)
}

pub(super) fn presentation_sections(package: &Package, main: &str) -> Result<Vec<Section>, String> {
    let xml = package.xml(main)?;
    let relationships = package.relationships(main)?;
    let mut sections = Vec::new();
    for (index, slide) in xml.named("sldId").enumerate() {
        let id = relationship_id(&xml, slide)?;
        let target = relationships
            .iter()
            .find(|relation| relation.id == id && relation.kind == "slide" && !relation.external)
            .ok_or("PowerPoint 缺少幻灯片关系")?;
        let number = index + 1;
        sections.push(Section {
            id: target.target.clone(),
            label: format!("第 {number} 页"),
            kind: "slide",
        });
        if let Some(notes) = package.related(&target.target, "notesSlide")? {
            sections.push(Section {
                id: notes,
                label: format!("第 {number} 页备注"),
                kind: "notes",
            });
        }
    }
    Ok(sections)
}

pub(super) fn relationship_id<'a>(xml: &'a Xml<'_>, index: usize) -> Result<&'a str, String> {
    xml.elements[index]
        .attrs
        .iter()
        .find(|(name, _)| name.ends_with(":id"))
        .map(|(_, value)| value.as_str())
        .ok_or_else(|| "Office 对象缺少关系标识".to_owned())
}

pub(super) fn items(
    package: &Package,
    section: &Section,
    offset: usize,
    limit: usize,
) -> Result<(usize, Vec<Item>), String> {
    let xml = package.xml(&section.id)?;
    let paragraphs: Vec<_> = xml.named("p").collect();
    let mut total = paragraphs.len();
    let mut items = Vec::new();
    for (ordinal, &paragraph) in paragraphs.iter().enumerate().skip(offset).take(limit) {
        let fragments = fragments(&xml, paragraph)?;
        let content: String = fragments
            .iter()
            .map(|(_, content)| content.as_str())
            .collect();
        let cell = xml
            .ancestors(paragraph)
            .find(|&index| xml.elements[index].local() == "tc");
        let style = xml
            .child(paragraph, "pPr")
            .and_then(|properties| xml.child(properties, "pStyle"))
            .and_then(|index| xml.elements[index].local_attr("val"));
        let kind = if cell.is_some() {
            "table_cell"
        } else if style.is_some_and(|name| name.to_ascii_lowercase().starts_with("heading")) {
            "heading"
        } else if section.kind == "document" {
            "paragraph"
        } else {
            "shape_text"
        };
        let label = match (section.kind, cell) {
            ("document", Some(cell)) => {
                let row = xml.elements[cell].parent.ok_or("Word 表格缺少行")?;
                let table = xml.elements[row].parent.ok_or("Word 表格缺少表格部件")?;
                let row_no = xml
                    .children(table, "tr")
                    .position(|index| index == row)
                    .map_or(0, |index| index + 1);
                let column = xml
                    .children(row, "tc")
                    .position(|index| index == cell)
                    .map_or(0, |index| index + 1);
                format!("表格 {row_no} 行 {column} 列")
            }
            ("document", None) => {
                style.map_or_else(|| format!("段落 {}", ordinal + 1), str::to_owned)
            }
            (_, _) => xml
                .ancestors(paragraph)
                .find(|&index| matches!(xml.elements[index].local(), "sp" | "graphicFrame"))
                .and_then(|shape| {
                    xml.descendants(shape)
                        .find(|&index| xml.elements[index].local() == "cNvPr")
                })
                .and_then(|index| xml.elements[index].attr("name"))
                .map_or_else(|| format!("文本 {}", ordinal + 1), str::to_owned),
        };
        let editable = !content.is_empty() && !protected(&xml, paragraph);
        items.push(Item::new(
            format!("p:{}", ordinal + 1),
            label,
            kind,
            content,
            editable,
        ));
    }
    if section.kind == "slide" {
        let mut objects = super::chart::presentation_items(package, section)?;
        if xml.named("timing").next().is_none() {
            for shape in xml.named("sp").filter(|&shape| {
                xml.elements[shape]
                    .parent
                    .is_some_and(|parent| xml.elements[parent].local() == "spTree")
            }) {
                let Some(info) = xml
                    .child(shape, "nvSpPr")
                    .and_then(|nv| xml.child(nv, "cNvPr"))
                else {
                    continue;
                };
                let Some(id) = xml.elements[info].attr("id") else {
                    continue;
                };
                let Some(offset) = xml
                    .child(shape, "spPr")
                    .and_then(|prop| xml.child(prop, "xfrm"))
                    .and_then(|transform| xml.child(transform, "off"))
                else {
                    continue;
                };
                let offset = &xml.elements[offset];
                let (Some(x), Some(y)) = (offset.attr("x"), offset.attr("y")) else {
                    continue;
                };
                let name = xml.elements[info].attr("name").unwrap_or("形状");
                objects.push(Item::new(
                    format!("shape:{id}"),
                    format!("{name}位置"),
                    "shape_position",
                    format!("{x}, {y}"),
                    true,
                ));
            }
        }
        let relations = package.relationships(&section.id)?;
        for picture in xml.named("pic") {
            let Some(info) = xml
                .child(picture, "nvPicPr")
                .and_then(|nv| xml.child(nv, "cNvPr"))
            else {
                continue;
            };
            let Some(id) = xml.elements[info].attr("id") else {
                continue;
            };
            let name = xml.elements[info].attr("name").unwrap_or("图片");
            let target = picture_relationship(&xml, picture).and_then(|id| {
                relations.iter().find(|relation| {
                    relation.id == id && relation.kind == "image" && !relation.external
                })
            });
            let extension = target.and_then(|relation| picture_type(&relation.target));
            objects.push(Item::new(
                format!("picture:{id}"),
                name.to_owned(),
                "picture",
                extension.map_or_else(
                    || "此图片不可替换".to_owned(),
                    |kind| format!("使用同格式 {kind} 图片替换"),
                ),
                extension.is_some(),
            ));
        }
        let object_count = objects.len();
        items.extend(
            objects
                .into_iter()
                .skip(offset.saturating_sub(total))
                .take(limit.saturating_sub(items.len())),
        );
        total += object_count;
    }
    Ok((total, items))
}

fn fragments(xml: &Xml<'_>, paragraph: usize) -> Result<Vec<(usize, String)>, String> {
    let mut fragments = Vec::new();
    for index in xml.descendants(paragraph) {
        if xml
            .ancestors(index)
            .take_while(|&parent| parent != paragraph)
            .any(|parent| matches!(xml.elements[parent].local(), "p" | "del" | "moveFrom"))
        {
            continue;
        }
        match xml.elements[index].local() {
            "t" => fragments.push((index, xml.text(index)?)),
            "tab" => fragments.push((index, "\t".to_owned())),
            "br" | "cr" => fragments.push((index, "\n".to_owned())),
            _ => {}
        }
    }
    Ok(fragments)
}

fn protected(xml: &Xml<'_>, paragraph: usize) -> bool {
    let has_protected_ancestor = xml.ancestors(paragraph).any(|index| {
        matches!(
            xml.elements[index].local(),
            "sdt" | "ins" | "del" | "moveFrom" | "moveTo" | "AlternateContent"
        ) || (xml.elements[index].local() == "tc"
            && xml.child(index, "tcPr").is_some_and(|properties| {
                xml.child(properties, "vMerge").is_some()
                    || xml.child(properties, "gridSpan").is_some()
            }))
    });
    has_protected_ancestor
        || xml.descendants(paragraph).any(|index| {
            matches!(
                xml.elements[index].local(),
                "fld"
                    | "fldChar"
                    | "instrText"
                    | "fldSimple"
                    | "sdt"
                    | "ins"
                    | "del"
                    | "moveFrom"
                    | "moveTo"
                    | "bookmarkStart"
                    | "bookmarkEnd"
                    | "commentRangeStart"
                    | "commentRangeEnd"
                    | "hyperlink"
                    | "AlternateContent"
            )
        })
}

pub(super) fn replace(
    package: &mut Package,
    section: &Section,
    item: &str,
    before: &str,
    after: &str,
) -> Result<Change, String> {
    validate_text(before)?;
    validate_text(after)?;
    if before.is_empty() || before == after || after.contains(['\n', '\r', '\t']) {
        return Err("请提供不同的原文与替换内容，替换内容应为单段文字".into());
    }
    let ordinal: usize = item
        .strip_prefix("p:")
        .and_then(|id| id.parse().ok())
        .filter(|number| *number > 0)
        .ok_or("Office 文本定位无效")?;
    let xml = package.xml(&section.id)?;
    let paragraph = xml
        .named("p")
        .nth(ordinal - 1)
        .ok_or("Office 文本定位已失效")?;
    if protected(&xml, paragraph) {
        return Err("选区包含域、批注、修订、书签或受保护结构".into());
    }
    let fragments = fragments(&xml, paragraph)?;
    let content: String = fragments.iter().map(|(_, text)| text.as_str()).collect();
    let mut positions = content
        .char_indices()
        .map(|(index, _)| index)
        .filter(|&index| content[index..].starts_with(before));
    let start = positions.next().ok_or("Office 选区与原文不匹配")?;
    if positions.next().is_some() {
        return Err("Office 选区原文出现多次，请提供更精确的文字".into());
    }
    let end = start + before.len();
    let mut position = 0;
    let mut changes = Vec::new();
    let mut style = None;
    for (index, text) in fragments {
        let next = position + text.len();
        if position < end && start < next {
            let node = &xml.elements[index];
            let run = node.parent.ok_or("Office 文字缺少文本片段")?;
            if node.local() != "t" || xml.elements[run].local() != "r" {
                return Err("选区跨越换行、制表符或特殊文本".into());
            }
            let properties = xml
                .child(run, "rPr")
                .map(|prop| &xml.source[xml.elements[prop].outer.clone()]);
            if let Some(first) = style {
                if properties != first {
                    return Err("选区跨越不同字符样式，请分段修改".into());
                }
            } else {
                style = Some(properties);
            }
            let from = start.saturating_sub(position);
            let to = (end - position).min(text.len());
            let replacement = if changes.is_empty() { after } else { "" };
            let value = format!("{}{replacement}{}", &text[..from], &text[to..]);
            let tag = if section.kind == "document"
                && (value.starts_with(char::is_whitespace) || value.ends_with(char::is_whitespace))
            {
                node.tag(&[("xml:space", "preserve")], false)
            } else {
                xml.source[node.outer.start..node.inner.start].to_owned()
            };
            changes.push((
                node.outer.clone(),
                format!("{tag}{}</{}>", escape(&value), node.name),
            ));
        }
        position = next;
    }
    let edited = replace_ranges(xml.source, changes);
    Xml::parse(&edited)?;
    package.set(&section.id, edited);
    Ok(Change {
        section: section.label.clone(),
        item: item.to_owned(),
        before: before.to_owned(),
        after: after.to_owned(),
    })
}

pub(super) fn move_shape(
    package: &mut Package,
    main: &str,
    section: &Section,
    item: &str,
    x: u32,
    y: u32,
) -> Result<Change, String> {
    if section.kind != "slide" {
        return Err("只能移动幻灯片上的形状".into());
    }
    let id = item.strip_prefix("shape:").ok_or("形状定位无效")?;
    let main = package.xml(main)?;
    let size = main.child(0, "sldSz").ok_or("演示缺少页面尺寸")?;
    let size = &main.elements[size];
    let number = |value: Option<&str>| {
        value
            .and_then(|value| value.parse::<u32>().ok())
            .ok_or("形状或页面坐标无效")
    };
    let bounds = (number(size.attr("cx"))?, number(size.attr("cy"))?);
    let slide = package.xml(&section.id)?;
    if slide.named("timing").next().is_some() {
        return Err("含动画的页面暂不支持移动形状".into());
    }
    let shape = slide
        .named("sp")
        .find(|&shape| {
            slide.elements[shape]
                .parent
                .is_some_and(|parent| slide.elements[parent].local() == "spTree")
                && slide
                    .child(shape, "nvSpPr")
                    .and_then(|nv| slide.child(nv, "cNvPr"))
                    .is_some_and(|node| slide.elements[node].attr("id") == Some(id))
        })
        .ok_or("只能移动独立的普通形状")?;
    let transform = slide
        .child(shape, "spPr")
        .and_then(|prop| slide.child(prop, "xfrm"))
        .ok_or("该形状使用继承位置，暂不可移动")?;
    let extent = slide.child(transform, "ext").ok_or("形状缺少尺寸")?;
    let extent = &slide.elements[extent];
    if x.saturating_add(number(extent.attr("cx"))?) > bounds.0
        || y.saturating_add(number(extent.attr("cy"))?) > bounds.1
    {
        return Err("移动后形状超出幻灯片".into());
    }
    let offset = slide.child(transform, "off").ok_or("形状缺少位置")?;
    let offset = &slide.elements[offset];
    let before = format!(
        "{}, {}",
        number(offset.attr("x"))?,
        number(offset.attr("y"))?
    );
    let after = format!("{x}, {y}");
    let replacement = offset.tag(&[("x", &x.to_string()), ("y", &y.to_string())], true);
    let edited = replace_ranges(slide.source, vec![(offset.outer.clone(), replacement)]);
    package.set(&section.id, edited);
    Ok(Change {
        section: section.label.clone(),
        item: item.to_owned(),
        before,
        after,
    })
}

fn picture_relationship<'a>(xml: &'a Xml<'_>, picture: usize) -> Option<&'a str> {
    let blip = xml
        .child(picture, "blipFill")
        .and_then(|fill| xml.child(fill, "blip"))?;
    xml.elements[blip].local_attr("embed")
}

fn picture_type(path: &str) -> Option<&'static str> {
    match path.rsplit_once('.')?.1.to_ascii_lowercase().as_str() {
        "png" => Some("PNG"),
        "jpg" | "jpeg" => Some("JPEG"),
        _ => None,
    }
}

pub(super) fn replace_picture(
    package: &mut Package,
    section: &Section,
    item: &str,
    data: &str,
) -> Result<Change, String> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    if section.kind != "slide" || data.len() > 6 * 1024 * 1024 {
        return Err("图片替换需要幻灯片上的图片，且文件不超过 4 MiB".into());
    }
    let id = item.strip_prefix("picture:").ok_or("图片定位无效")?;
    let xml = package.xml(&section.id)?;
    let picture = xml
        .named("pic")
        .find(|&index| {
            xml.child(index, "nvPicPr")
                .and_then(|nv| xml.child(nv, "cNvPr"))
                .is_some_and(|node| xml.elements[node].attr("id") == Some(id))
        })
        .ok_or("幻灯片上找不到该图片")?;
    let relationship = picture_relationship(&xml, picture).ok_or("图片没有本地嵌入数据")?;
    if xml
        .named("blip")
        .filter(|&index| xml.elements[index].local_attr("embed") == Some(relationship))
        .count()
        != 1
    {
        return Err("同页共享图片暂不可替换".into());
    }
    let part = package
        .relationships(&section.id)?
        .into_iter()
        .find(|rel| rel.id == relationship && rel.kind == "image" && !rel.external)
        .ok_or("图片缺少内部关系")?
        .target;
    if super::chart::relationship_count(package, &part)? != 1 {
        return Err("跨页共享图片暂不可替换".into());
    }
    let kind = picture_type(&part).ok_or("仅支持同格式 PNG 或 JPEG 替换")?;
    let bytes = STANDARD.decode(data).map_err(|_| "图片数据无法解析")?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("图片超过 4 MiB".into());
    }
    let dimensions = if kind == "PNG"
        && bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        && bytes.get(12..16) == Some(b"IHDR")
    {
        bytes.get(16..24).map(|size| {
            (
                u32::from_be_bytes(size[..4].try_into().unwrap()),
                u32::from_be_bytes(size[4..].try_into().unwrap()),
            )
        })
    } else if kind == "JPEG" && bytes.starts_with(b"\xff\xd8") {
        jpeg_dimensions(&bytes)
    } else {
        None
    };
    if !dimensions
        .is_some_and(|(width, height)| width > 0 && height > 0 && width <= 8_192 && height <= 8_192)
    {
        return Err("图片类型与原图不一致，或像素尺寸超出支持范围".into());
    }
    let old = package.parts.get(&part).ok_or("图片部件缺失")?.len();
    let new = bytes.len();
    package.set(part, bytes);
    Ok(Change {
        section: section.label.clone(),
        item: item.to_owned(),
        before: format!("{kind}，{old} 字节"),
        after: format!("{kind}，{new} 字节"),
    })
}

fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut cursor = 2;
    while bytes.get(cursor) == Some(&0xff) {
        let marker = *bytes.get(cursor + 1)?;
        if marker == 0xff {
            cursor += 1;
            continue;
        }
        if matches!(marker, 0xd9 | 0xda) {
            return None;
        }
        let size = u16::from_be_bytes(bytes.get(cursor + 2..cursor + 4)?.try_into().ok()?) as usize;
        if size < 2 {
            return None;
        }
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
            let frame = bytes.get(cursor + 4..cursor + 9)?;
            let height = u16::from_be_bytes(frame[1..3].try_into().ok()?);
            let width = u16::from_be_bytes(frame[3..5].try_into().ok()?);
            return Some((u32::from(width), u32::from(height)));
        }
        cursor += size + 2;
    }
    None
}
