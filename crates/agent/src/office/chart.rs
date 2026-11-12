use super::xml::{escape, replace_ranges, validate_text, Xml};
use super::{CellValue, Change, Edit, Format, Item, OfficeFile, Package, Section};
use serde::{Deserialize, Serialize};

const C: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartKind {
    Column,
    Bar,
    Line,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SlideChart {
    pub kind: ChartKind,
    pub title: String,
    pub categories: Vec<String>,
    pub values: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ChartValues {
    pub categories: Vec<String>,
    pub values: Vec<String>,
}

impl ChartValues {
    pub(super) fn byte_len(&self) -> usize {
        self.categories
            .iter()
            .chain(&self.values)
            .map(String::len)
            .sum()
    }
}

pub(super) fn check_data(categories: &[String], values: &[String]) -> Result<(), String> {
    if categories.is_empty() || categories.len() > 300 || categories.len() != values.len() {
        return Err("图表需要 1 至 300 组对应的分类和数值".into());
    }
    for category in categories {
        validate_text(category)?;
        if category.chars().count() > 200 {
            return Err("图表分类名称过长".into());
        }
    }
    if values
        .iter()
        .any(|value| !value.parse::<f64>().is_ok_and(f64::is_finite))
    {
        return Err("图表数据必须是有限数字".into());
    }
    Ok(())
}

pub(super) fn points(prefix: &str, data: &[String]) -> String {
    let mut result = format!(r#"<{prefix}ptCount val="{}"/>"#, data.len());
    for (index, value) in data.iter().enumerate() {
        result.push_str(&format!(
            r#"<{prefix}pt idx="{index}"><{prefix}v>{}</{prefix}v></{prefix}pt>"#,
            escape(value)
        ));
    }
    result
}

pub(super) fn xml(
    spec: &SlideChart,
    category_range: &str,
    value_range: &str,
    embedded: bool,
) -> Result<String, String> {
    validate_text(&spec.title)?;
    check_data(&spec.categories, &spec.values)?;
    let title = escape(&spec.title);
    let categories = points("c:", &spec.categories);
    let values = points("c:", &spec.values);
    let category_range = escape(category_range);
    let value_range = escape(value_range);
    let series = format!(
        r#"<c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:v>{title}</c:v></c:tx><c:spPr><a:solidFill><a:srgbClr val="245785"/></a:solidFill></c:spPr><c:cat><c:strRef><c:f>{category_range}</c:f><c:strCache>{categories}</c:strCache></c:strRef></c:cat><c:val><c:numRef><c:f>{value_range}</c:f><c:numCache><c:formatCode>General</c:formatCode>{values}</c:numCache></c:numRef></c:val></c:ser>"#
    );
    let axes = r#"<c:axId val="9001"/><c:axId val="9002"/>"#;
    let chart = match spec.kind {
        ChartKind::Line => {
            format!(r#"<c:lineChart><c:grouping val="standard"/>{series}{axes}</c:lineChart>"#)
        }
        ChartKind::Column | ChartKind::Bar => {
            let direction = if spec.kind == ChartKind::Column {
                "col"
            } else {
                "bar"
            };
            format!(
                r#"<c:barChart><c:barDir val="{direction}"/><c:grouping val="clustered"/>{series}<c:gapWidth val="85"/>{axes}</c:barChart>"#
            )
        }
    };
    let (categories_side, values_side) = if spec.kind == ChartKind::Bar {
        ("l", "b")
    } else {
        ("b", "l")
    };
    let link = if embedded {
        r#"<c:externalData r:id="rId1"><c:autoUpdate val="0"/></c:externalData>"#
    } else {
        ""
    };
    Ok(format!(
        r#"<c:chartSpace xmlns:c="{C}" xmlns:a="{A}" xmlns:r="{REL}"><c:chart><c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>{title}</a:t></a:r></a:p></c:rich></c:tx><c:layout/><c:overlay val="0"/></c:title><c:plotArea><c:layout/>{chart}<c:catAx><c:axId val="9001"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="{categories_side}"/><c:tickLblPos val="nextTo"/><c:crossAx val="9002"/><c:crosses val="autoZero"/><c:auto val="1"/><c:lblAlgn val="ctr"/><c:lblOffset val="100"/></c:catAx><c:valAx><c:axId val="9002"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="{values_side}"/><c:majorGridlines/><c:numFmt formatCode="General" sourceLinked="1"/><c:tickLblPos val="nextTo"/><c:crossAx val="9001"/><c:crosses val="autoZero"/><c:crossBetween val="between"/></c:valAx></c:plotArea><c:plotVisOnly val="1"/><c:dispBlanksAs val="gap"/></c:chart>{link}</c:chartSpace>"#
    ))
}

fn reference<'a>(
    xml: &'a Xml<'_>,
    node: usize,
    container: &str,
    reference: &str,
    cache: &str,
) -> Result<(String, usize), String> {
    let container = xml.child(node, container).ok_or("图表缺少数据系列")?;
    let reference = xml
        .child(container, reference)
        .ok_or("当前图表数据引用类型暂不可编辑")?;
    let formula = xml.child(reference, "f").ok_or("图表缺少来源范围")?;
    let cache = xml.child(reference, cache).ok_or("图表缺少可编辑缓存")?;
    Ok((xml.text(formula)?, cache))
}

fn cache_values(xml: &Xml<'_>, cache: usize) -> Result<Vec<String>, String> {
    xml.children(cache, "pt")
        .map(|point| {
            let node = xml.child(point, "v").ok_or("图表数据点缺少值")?;
            xml.text(node)
        })
        .collect()
}

pub(super) fn presentation_items(
    package: &Package,
    section: &Section,
) -> Result<Vec<Item>, String> {
    if section.kind != "slide" {
        return Ok(Vec::new());
    }
    let mut items = Vec::new();
    for relation in package
        .relationships(&section.id)?
        .into_iter()
        .filter(|relation| !relation.external && relation.kind == "chart")
    {
        let xml = package.xml(&relation.target)?;
        let mut description = Vec::new();
        for name in ["strRef", "numRef"] {
            for index in xml.named(name) {
                if let Some(formula) = xml.child(index, "f") {
                    description.push(xml.text(formula)?);
                }
                for cache in ["strCache", "numCache"] {
                    if let Some(cache) = xml.child(index, cache) {
                        description.push(cache_values(&xml, cache)?.join(" / "));
                    }
                }
            }
        }
        let title = xml
            .named("title")
            .next()
            .map(|title| {
                xml.descendants(title)
                    .filter(|&index| xml.elements[index].local() == "t")
                    .filter_map(|index| xml.text(index).ok())
                    .collect::<String>()
            })
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| "原生图表".into());
        let mut item = Item::new(
            format!("chart:{}", relation.id),
            title,
            "chart",
            description.join("\n"),
            false,
        );
        if let Some(chart) = editable_values(package, &relation.target, &xml) {
            item.editable = true;
            item.chart = Some(chart);
        }
        items.push(item);
    }
    Ok(items)
}

fn editable_values(package: &Package, part: &str, xml: &Xml<'_>) -> Option<ChartValues> {
    let mut series = xml.named("ser");
    let node = series.next()?;
    if series.next().is_some() || package.related(part, "package").ok().flatten().is_none() {
        return None;
    }
    let (_, category) = reference(xml, node, "cat", "strRef", "strCache").ok()?;
    let (_, value) = reference(xml, node, "val", "numRef", "numCache").ok()?;
    let categories = cache_values(xml, category).ok()?;
    let values = cache_values(xml, value).ok()?;
    check_data(&categories, &values).ok()?;
    let chart = ChartValues { categories, values };
    (chart.byte_len() <= 6000).then_some(chart)
}

pub(super) fn relationship_count(package: &Package, target: &str) -> Result<usize, String> {
    let mut count = 0;
    for part in package.parts.keys().filter(|part| part.ends_with(".rels")) {
        let owner = if part == "_rels/.rels" {
            String::new()
        } else if let Some((directory, name)) = part.rsplit_once("/_rels/") {
            format!("{directory}/{}", name.strip_suffix(".rels").unwrap())
        } else {
            continue;
        };
        count += package
            .relationships(&owner)?
            .iter()
            .filter(|relation| !relation.external && relation.target == target)
            .count();
    }
    Ok(count)
}

pub(super) fn update_presentation(
    package: &mut Package,
    section: &Section,
    item: &str,
    categories: &[String],
    values: &[String],
) -> Result<Change, String> {
    check_data(categories, values)?;
    let id = item.strip_prefix("chart:").ok_or("图表定位无效")?;
    let relation = package
        .relationships(&section.id)?
        .into_iter()
        .find(|relation| !relation.external && relation.kind == "chart" && relation.id == id)
        .ok_or("幻灯片中找不到指定图表")?;
    let part = relation.target;
    let embedded = package
        .related(&part, "package")?
        .ok_or("图表没有可写的嵌入工作簿")?;
    if relationship_count(package, &part)? != 1 || relationship_count(package, &embedded)? != 1 {
        return Err("共享图表或共享嵌入工作簿暂不可编辑".into());
    }
    let xml = package.xml(&part)?;
    let mut series = xml.named("ser");
    let series_id = series.next().ok_or("图表缺少数据系列")?;
    if series.next().is_some() {
        return Err("首版支持单系列图表数据更新".into());
    }
    drop(series);
    let (category_ref, category_cache) = reference(&xml, series_id, "cat", "strRef", "strCache")?;
    let (value_ref, value_cache) = reference(&xml, series_id, "val", "numRef", "numCache")?;
    let before = cache_values(&xml, value_cache)?.join(" / ");
    let mut edits = Vec::new();
    for (reference, data, numeric) in [
        (&category_ref, categories, false),
        (&value_ref, values, true),
    ] {
        let (name, addresses) = super::sheet::chart_range(reference)?;
        if addresses.len() != data.len() {
            return Err("图表数据项数量必须与现有系列一致".into());
        }
        for (address, value) in addresses.into_iter().zip(data) {
            edits.push(Edit::SetCell {
                section: name.clone(),
                address,
                value: if numeric {
                    CellValue::Number(value.clone())
                } else {
                    CellValue::Text(value.clone())
                },
            });
        }
    }
    let original = package.parts.get(&embedded).ok_or("缺少嵌入工作簿")?;
    let file = OfficeFile::from_bytes(Format::Xlsx, original)?;
    let version = file.version.clone();
    let draft = file.edit(&version, &edits)?;
    if draft.needs_calculation {
        return Err("嵌入工作簿仍需复算".into());
    }
    let updated = replace_ranges(
        xml.source,
        vec![
            cache_update(&xml, category_cache, categories),
            cache_update(&xml, value_cache, values),
        ],
    );
    package.set(part, updated);
    package.set(embedded, draft.bytes);
    Ok(Change {
        section: section.label.clone(),
        item: item.to_owned(),
        before,
        after: values.join(" / "),
    })
}

pub(super) fn cache_update(
    xml: &Xml<'_>,
    cache: usize,
    data: &[String],
) -> (std::ops::Range<usize>, String) {
    let node = &xml.elements[cache];
    let prefix = node
        .name
        .rsplit_once(':')
        .map_or(String::new(), |(prefix, _)| format!("{prefix}:"));
    let mut value = String::new();
    if let Some(format) = xml.child(cache, "formatCode") {
        value.push_str(&xml.source[xml.elements[format].outer.clone()]);
    }
    value.push_str(&points(&prefix, data));
    if let Some(extension) = xml.child(cache, "extLst") {
        value.push_str(&xml.source[xml.elements[extension].outer.clone()]);
    }
    (node.inner.clone(), value)
}
