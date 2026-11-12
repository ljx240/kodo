use super::text::relationship_id;
use super::xml::{escape, replace_ranges, validate_text, Xml};
use super::{CellValue, Change, Item, Package, Section};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[path = "calc.rs"]
mod calc;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CellDetails {
    pub value_type: String,
    pub formula: Option<String>,
    pub cache: Option<&'static str>,
    pub number_format: Option<String>,
    pub hidden: bool,
}

impl CellDetails {
    pub(super) fn byte_len(&self) -> usize {
        self.formula.as_ref().map_or(0, String::len)
            + self.number_format.as_ref().map_or(0, String::len)
    }
}

pub(super) fn sections(package: &Package, main: &str) -> Result<Vec<Section>, String> {
    let xml = package.xml(main)?;
    let relationships = package.relationships(main)?;
    xml.named("sheet")
        .filter_map(|index| {
            let id = match relationship_id(&xml, index) {
                Ok(id) => id,
                Err(error) => return Some(Err(error)),
            };
            let part = relationships
                .iter()
                .find(|relation| relation.id == id && !relation.external)?;
            if part.kind != "worksheet" {
                return None;
            }
            let node = &xml.elements[index];
            let Some(name) = node
                .attr("name")
                .filter(|name| (1..=31).contains(&name.chars().count()))
            else {
                return Some(Err("Excel 工作表名称无效".into()));
            };
            Some(Ok(Section {
                id: part.target.clone(),
                label: name.to_owned(),
                kind: if matches!(node.attr("state"), Some("hidden" | "veryHidden")) {
                    "hidden_sheet"
                } else {
                    "sheet"
                },
            }))
        })
        .collect()
}

fn cells<'a>(xml: &'a Xml<'_>) -> impl Iterator<Item = usize> + 'a {
    xml.children(0, "sheetData")
        .flat_map(|data| xml.children(data, "row"))
        .flat_map(|row| xml.children(row, "c"))
}

fn coordinate(address: &str) -> Option<(u32, u32)> {
    let address = address.trim();
    let split = address.find(|character: char| !character.is_ascii_alphabetic())?;
    let (letters, digits) = address.split_at(split);
    if letters.is_empty()
        || letters.len() > 3
        || digits.is_empty()
        || digits.starts_with('0')
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let column = letters.bytes().fold(0, |value, letter| {
        value * 26 + u32::from(letter.to_ascii_uppercase() - b'A' + 1)
    });
    let row = digits.parse().ok()?;
    ((1..=1_048_576).contains(&row) && column <= 16_384).then_some((row, column))
}

fn parse_range(range: &str) -> Option<((u32, u32), (u32, u32))> {
    let clean = range.replace('$', "");
    let (start, end) = clean.split_once(':').unwrap_or((&clean, &clean));
    let (start, end) = (coordinate(start)?, coordinate(end)?);
    (start.0 <= end.0 && start.1 <= end.1).then_some((start, end))
}

fn within(cell: (u32, u32), range: ((u32, u32), (u32, u32))) -> bool {
    (range.0 .0..=range.1 .0).contains(&cell.0) && (range.0 .1..=range.1 .1).contains(&cell.1)
}

fn text_children(xml: &Xml<'_>, index: usize) -> Result<String, String> {
    let mut text = String::new();
    for item in xml.descendants(index).filter(|&item| {
        xml.elements[item].local() == "t"
            && !xml
                .ancestors(item)
                .take_while(|&ancestor| ancestor != index)
                .any(|ancestor| xml.elements[ancestor].local() == "rPh")
    }) {
        text.push_str(&xml.text(item)?);
    }
    Ok(text)
}

fn read_strings(package: &Package, main: &str) -> Result<Vec<String>, String> {
    let Some(part) = package.related(main, "sharedStrings")? else {
        return Ok(Vec::new());
    };
    let xml = package.xml(&part)?;
    xml.children(0, "si")
        .map(|index| text_children(&xml, index))
        .collect()
}

fn read_formats(package: &Package, main: &str) -> Result<Vec<String>, String> {
    let Some(part) = package.related(main, "styles")? else {
        return Ok(Vec::new());
    };
    let xml = package.xml(&part)?;
    let custom: BTreeMap<_, _> = xml
        .named("numFmt")
        .filter_map(|index| {
            let node = &xml.elements[index];
            Some((node.attr("numFmtId")?, node.attr("formatCode")?))
        })
        .collect();
    let mut styles = Vec::new();
    for index in xml
        .children(0, "cellXfs")
        .flat_map(|xfs| xml.children(xfs, "xf"))
    {
        let id = xml.elements[index].attr("numFmtId").unwrap_or("0");
        styles.push(
            custom
                .get(id)
                .map(|format| format.chars().take(500).collect())
                .unwrap_or_else(|| match id {
                    "0" => "General".to_owned(),
                    "1" => "0".to_owned(),
                    "2" => "0.00".to_owned(),
                    "3" => "#,##0".to_owned(),
                    "4" => "#,##0.00".to_owned(),
                    "9" => "0%".to_owned(),
                    "10" => "0.00%".to_owned(),
                    "14" => "m/d/yy".to_owned(),
                    _ => format!("Excel 格式 {id}"),
                }),
        );
    }
    Ok(styles)
}

fn merged_ranges(xml: &Xml<'_>) -> Vec<((u32, u32), (u32, u32))> {
    xml.named("mergeCell")
        .filter_map(|index| parse_range(xml.elements[index].attr("ref")?))
        .collect()
}

fn calculation_pending(workbook: &Xml<'_>) -> bool {
    workbook.child(0, "calcPr").is_some_and(|index| {
        matches!(
            workbook.elements[index].attr("fullCalcOnLoad"),
            Some("1" | "true")
        )
    })
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisRequest {
    pub columns: Vec<String>,
    pub group_by: Option<String>,
    #[serde(default)]
    pub header: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetAnalysis {
    pub sheet: String,
    pub version: String,
    pub range: String,
    pub rows: u32,
    pub header_skipped: bool,
    pub group_by: Option<String>,
    pub totals: BTreeMap<String, ColumnSummary>,
    pub groups: Vec<GroupSummary>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupSummary {
    group: GroupKey,
    rows: u32,
    cached_keys: u32,
    columns: BTreeMap<String, ColumnSummary>,
}

#[derive(Default, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnSummary {
    count: u32,
    blank: u32,
    non_numeric: u32,
    unavailable_formula: u32,
    cached_formula: u32,
    sum: Option<f64>,
    min: Option<f64>,
    max: Option<f64>,
    average: Option<f64>,
}

#[derive(Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
enum GroupKey {
    Blank,
    Unavailable,
    Other,
    Text(String),
    Boolean(bool),
    Number(String),
}

enum AnalysisValue {
    Blank,
    Unavailable,
    Other,
    Text(String),
    Boolean(bool),
    Number(f64),
}

impl ColumnSummary {
    fn record(&mut self, value: &AnalysisValue, cached: bool) -> Result<(), String> {
        match value {
            AnalysisValue::Blank => self.blank += 1,
            AnalysisValue::Unavailable => self.unavailable_formula += 1,
            AnalysisValue::Number(value) => {
                self.count += 1;
                self.cached_formula += u32::from(cached);
                let sum = self.sum.unwrap_or(0.0) + value;
                if !sum.is_finite() {
                    return Err("Excel 汇总超出数值范围".into());
                }
                self.sum = Some(sum);
                self.min = Some(self.min.map_or(*value, |min| min.min(*value)));
                self.max = Some(self.max.map_or(*value, |max| max.max(*value)));
                self.average = Some(sum / f64::from(self.count));
            }
            _ => self.non_numeric += 1,
        }
        Ok(())
    }
}

fn analysis_value(
    xml: &Xml<'_>,
    index: Option<&usize>,
    strings: &[String],
    pending: bool,
) -> Result<(AnalysisValue, bool), String> {
    let Some(&index) = index else {
        return Ok((AnalysisValue::Blank, false));
    };
    let formula = xml.child(index, "f").is_some();
    let text = cell_text(xml, index, strings)?;
    if formula && (pending || text.is_empty()) {
        return Ok((AnalysisValue::Unavailable, false));
    }
    if text.is_empty() {
        return Ok((AnalysisValue::Blank, false));
    }
    let value = match xml.elements[index].attr("t") {
        None | Some("n") => {
            let number = text
                .parse::<f64>()
                .ok()
                .filter(|number| number.is_finite())
                .ok_or("Excel 数值单元格内容无效")?;
            AnalysisValue::Number(number)
        }
        Some("s" | "str" | "inlineStr") => AnalysisValue::Text(text),
        Some("b") => AnalysisValue::Boolean(text == "1"),
        _ => AnalysisValue::Other,
    };
    Ok((value, formula))
}

pub(super) fn analyze(
    package: &Package,
    main: &str,
    section: &Section,
    version: &str,
    range: &str,
    request: &AnalysisRequest,
) -> Result<SheetAnalysis, String> {
    let ((start_row, start_col), (end_row, end_col)) =
        parse_range(range).ok_or("Excel 分析区域应为 A1:B5")?;
    if u64::from(end_row - start_row + 1) * u64::from(end_col - start_col + 1) > 50_000 {
        return Err("Excel 分析范围最多包含 50000 个单元格".into());
    }
    if !(1..=8).contains(&request.columns.len()) {
        return Err("Excel 分析需要 1 至 8 个数值列".into());
    }
    let parse_column = |name: &str| {
        coordinate(&format!("{}1", name.trim()))
            .filter(|(row, _)| *row == 1)
            .map(|(_, column)| column)
            .filter(|column| (start_col..=end_col).contains(column))
            .ok_or_else(|| format!("Excel 分析列 {name} 必须在指定范围内"))
    };
    let columns: BTreeMap<u32, String> = request
        .columns
        .iter()
        .map(|name| Ok((parse_column(name)?, name.trim().to_ascii_uppercase())))
        .collect::<Result<_, String>>()?;
    if columns.len() != request.columns.len() {
        return Err("Excel 分析列不能重复".into());
    }
    let group_column = request.group_by.as_deref().map(parse_column).transpose()?;
    let first = start_row + u32::from(request.header);
    let xml = package.xml(&section.id)?;
    let strings = read_strings(package, main)?;
    let pending = calculation_pending(&package.xml(main)?);
    let mut selected = BTreeMap::new();
    for index in cells(&xml) {
        let (row, column) = xml.elements[index]
            .attr("r")
            .and_then(coordinate)
            .ok_or("Excel 单元格地址无效")?;
        if (first..=end_row).contains(&row)
            && (columns.contains_key(&column) || group_column == Some(column))
        {
            selected.insert((row, column), index);
        }
    }
    let summaries = || {
        columns
            .values()
            .map(|name| (name.clone(), ColumnSummary::default()))
            .collect()
    };
    let mut totals: BTreeMap<String, ColumnSummary> = summaries();
    let mut groups: BTreeMap<GroupKey, (u32, u32, BTreeMap<String, ColumnSummary>)> =
        BTreeMap::new();
    for row in first..=end_row {
        let mut group = if let Some(column) = group_column {
            let (value, cached) =
                analysis_value(&xml, selected.get(&(row, column)), &strings, pending)?;
            let key = match value {
                AnalysisValue::Blank => GroupKey::Blank,
                AnalysisValue::Unavailable => GroupKey::Unavailable,
                AnalysisValue::Other => GroupKey::Other,
                AnalysisValue::Text(text) => {
                    if text.chars().count() > 100 {
                        return Err("Excel 分组值最多 100 字符".into());
                    }
                    GroupKey::Text(text)
                }
                AnalysisValue::Boolean(value) => GroupKey::Boolean(value),
                AnalysisValue::Number(value) => GroupKey::Number(value.to_string()),
            };
            if groups.len() == 20 && !groups.contains_key(&key) {
                return Err("Excel 分析最多支持 20 个分组，请缩小范围".into());
            }
            let (rows, cached_keys, stats) =
                groups.entry(key).or_insert_with(|| (0, 0, summaries()));
            *rows += 1;
            *cached_keys += u32::from(cached);
            Some(stats)
        } else {
            None
        };
        for (column, name) in &columns {
            let (value, cached) =
                analysis_value(&xml, selected.get(&(row, *column)), &strings, pending)?;
            totals.get_mut(name).unwrap().record(&value, cached)?;
            if let Some(stats) = group.as_deref_mut() {
                stats.get_mut(name).unwrap().record(&value, cached)?;
            }
        }
    }
    Ok(SheetAnalysis {
        sheet: section.label.clone(),
        version: version.to_owned(),
        range: range.to_owned(),
        rows: end_row + 1 - first,
        header_skipped: request.header,
        group_by: request
            .group_by
            .as_ref()
            .map(|name| name.trim().to_ascii_uppercase()),
        totals,
        groups: groups
            .into_iter()
            .map(|(group, (rows, cached_keys, columns))| GroupSummary {
                group,
                rows,
                cached_keys,
                columns,
            })
            .collect(),
    })
}

pub(super) fn items(
    package: &Package,
    main: &str,
    section: &Section,
    offset: usize,
    limit: usize,
    range: Option<&str>,
) -> Result<(usize, Vec<Item>), String> {
    let range = range
        .map(|value| parse_range(value).ok_or("Excel 区域应为 A1 或 A1:B5"))
        .transpose()?;
    let xml = package.xml(&section.id)?;
    let strings = read_strings(package, main)?;
    let formats = read_formats(package, main)?;
    let merged = merged_ranges(&xml);
    let protected = xml.named("sheetProtection").next().is_some();
    let pending = calculation_pending(&package.xml(main)?);
    let hidden_columns: Vec<_> = xml
        .named("col")
        .filter_map(|index| {
            let node = &xml.elements[index];
            if !matches!(node.attr("hidden"), Some("1" | "true")) {
                return None;
            }
            Some(node.attr("min")?.parse::<u32>().ok()?..=node.attr("max")?.parse::<u32>().ok()?)
        })
        .collect();
    let mut total = 0;
    let mut items = Vec::new();
    for index in cells(&xml) {
        let node = &xml.elements[index];
        let address = node.attr("r").ok_or("Excel 单元格缺少地址")?;
        let location = coordinate(address).ok_or("Excel 单元格地址无效")?;
        if range.is_some_and(|range| !within(location, range)) {
            continue;
        }
        total += 1;
        if total <= offset || items.len() >= limit {
            continue;
        }
        let value = cell_text(&xml, index, &strings)?;
        let formula = xml
            .child(index, "f")
            .map(|node| xml.text(node))
            .transpose()?;
        let format = node
            .attr("s")
            .and_then(|id| id.parse::<usize>().ok())
            .and_then(|id| formats.get(id))
            .cloned();
        let hidden = hidden_columns
            .iter()
            .any(|columns| columns.contains(&location.1))
            || node
                .parent
                .is_some_and(|row| matches!(xml.elements[row].attr("hidden"), Some("1" | "true")));
        let cache = formula.as_ref().map(|_| {
            if pending {
                "pending"
            } else if xml.child(index, "v").is_some() && !value.is_empty() {
                "cached"
            } else {
                "missing"
            }
        });
        let editable = !protected
            && !merged.iter().any(|range| within(location, *range))
            && xml
                .child(index, "f")
                .is_none_or(|formula| xml.elements[formula].attr("t").is_none());
        let clipped_formula = formula
            .as_ref()
            .is_some_and(|text| text.chars().nth(2_000).is_some());
        let mut item = Item::new(
            address.to_owned(),
            address.to_owned(),
            "cell",
            value,
            editable && !clipped_formula,
        );
        item.clipped |= clipped_formula;
        item.cell = Some(CellDetails {
            value_type: match node.attr("t") {
                Some("s" | "str" | "inlineStr") => "text",
                Some("b") => "boolean",
                Some("d") => "date",
                Some("e") => "error",
                _ if item.text.is_empty() => "blank",
                _ => "number",
            }
            .to_owned(),
            formula: formula.map(|formula| formula.chars().take(2_000).collect()),
            cache,
            number_format: format,
            hidden,
        });
        items.push(item);
    }
    Ok((total, items))
}

fn cell_text(xml: &Xml<'_>, index: usize, strings: &[String]) -> Result<String, String> {
    if xml.elements[index].attr("t") == Some("inlineStr") {
        return xml
            .child(index, "is")
            .map_or_else(|| Ok(String::new()), |part| text_children(xml, part));
    }
    let raw = xml
        .child(index, "v")
        .map(|node| xml.text(node))
        .transpose()?
        .unwrap_or_default();
    if xml.elements[index].attr("t") == Some("s") {
        let string = raw
            .parse::<usize>()
            .ok()
            .and_then(|index| strings.get(index))
            .ok_or("Excel 共享字符串引用无效")?;
        return Ok(string.clone());
    }
    Ok(raw)
}

pub(super) fn cell_parts(
    value: &CellValue,
    prefix: &str,
) -> Result<(&'static str, String, String), String> {
    let (kind, content, label) = match value {
        CellValue::Text(value) => {
            validate_text(value)?;
            (
                "inlineStr",
                format!(
                    "<{prefix}is><{prefix}t xml:space=\"preserve\">{}</{prefix}t></{prefix}is>",
                    escape(value)
                ),
                value.clone(),
            )
        }
        CellValue::Number(value) => {
            if value
                .parse::<f64>()
                .ok()
                .filter(|number| number.is_finite())
                .is_none()
            {
                return Err("Excel 数值必须是有限数字".into());
            }
            (
                "n",
                format!("<{prefix}v>{value}</{prefix}v>"),
                value.clone(),
            )
        }
        CellValue::Boolean(value) => (
            "b",
            format!("<{prefix}v>{}</{prefix}v>", u8::from(*value)),
            value.to_string(),
        ),
        CellValue::Date(value) => {
            let segments: Vec<_> = value.split('-').collect();
            let valid = segments.len() == 3
                && segments.iter().map(|part| part.len()).eq([4, 2, 2])
                && segments
                    .iter()
                    .all(|part| part.bytes().all(|byte| byte.is_ascii_digit()))
                && segments[0] != "0000"
                && segments[1]
                    .parse::<u8>()
                    .is_ok_and(|month| (1..=12).contains(&month))
                && segments[2]
                    .parse::<u8>()
                    .is_ok_and(|day| (1..=31).contains(&day));
            if !valid {
                return Err("Excel 日期必须使用 YYYY-MM-DD".into());
            }
            (
                "d",
                format!("<{prefix}v>{value}</{prefix}v>"),
                value.clone(),
            )
        }
        CellValue::Formula(value) => {
            validate_text(value)?;
            let value = value.strip_prefix('=').unwrap_or(value);
            let functions_allowed = value.split('(').rev().skip(1).all(|part| {
                let name = part
                    .trim_end()
                    .rsplit(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '.')
                    .next()
                    .unwrap_or("")
                    .to_ascii_uppercase();
                name.is_empty()
                    || name.bytes().all(|byte| byte.is_ascii_digit())
                    || matches!(
                        name.as_str(),
                        "SUM" | "COUNT" | "AVERAGE" | "IF" | "SUMIFS" | "COUNTIFS"
                    )
            });
            if value.trim().is_empty()
                || value.len() > 8192
                || value.contains(['[', ']', '|'])
                || !functions_allowed
            {
                return Err("Excel 公式为空或包含当前不支持的函数、外部引用".into());
            }
            (
                "n",
                format!("<{prefix}f>{}</{prefix}f>", escape(value)),
                format!("={value}"),
            )
        }
        CellValue::Blank => ("n", String::new(), String::new()),
    };
    Ok((kind, content, label))
}

pub(super) fn set_cell(
    package: &mut Package,
    section: &Section,
    address: &str,
    value: &CellValue,
) -> Result<Change, String> {
    let address = address.trim().to_ascii_uppercase();
    let location = coordinate(&address).ok_or("Excel 单元格地址无效")?;
    let xml = package.xml(&section.id)?;
    if xml.named("sheetProtection").next().is_some()
        || merged_ranges(&xml)
            .iter()
            .any(|range| within(location, *range))
    {
        return Err("Excel 合并区或受保护的单元格暂不可编辑".into());
    }
    if package
        .parts
        .keys()
        .any(|part| part.contains("/pivotTable") || part.ends_with("/connections.xml"))
    {
        return Err("工作簿含透视表或数据连接，当前无法证明此次修改的影响范围".into());
    }
    let data = xml
        .child(0, "sheetData")
        .ok_or("Excel 工作表缺少单元格部件")?;
    let data_node = &xml.elements[data];
    let prefix = data_node.name.strip_suffix("sheetData").unwrap();
    let (kind, content, after) = cell_parts(value, prefix)?;
    let row = xml.children(data, "row").find(|&index| {
        xml.elements[index]
            .attr("r")
            .and_then(|value| value.parse().ok())
            == Some(location.0)
    });
    let existing = row.and_then(|row| {
        xml.children(row, "c")
            .find(|&index| xml.elements[index].attr("r").and_then(coordinate) == Some(location))
    });
    let mut before = String::new();
    let (target, replacement) = if let Some(index) = existing {
        let node = &xml.elements[index];
        let formula = xml.child(index, "f");
        if formula.is_some_and(|formula| xml.elements[formula].attr("t").is_some())
            || node
                .children
                .iter()
                .any(|&child| !matches!(xml.elements[child].local(), "v" | "f" | "is"))
        {
            return Err("该 Excel 单元格包含特殊公式或扩展对象".into());
        }
        let workbook = package
            .relationships("")?
            .into_iter()
            .find(|relation| relation.kind == "officeDocument")
            .ok_or("Excel 缺少工作簿关系")?
            .target;
        before = if let Some(formula) = formula {
            format!("={}", xml.text(formula)?)
        } else {
            cell_text(&xml, index, &read_strings(package, &workbook)?)?
        };
        (
            node.outer.clone(),
            format!(
                "{}{content}</{}>",
                node.tag(&[("t", kind)], false),
                node.name
            ),
        )
    } else {
        let new_cell = format!("<{prefix}c r=\"{address}\" t=\"{kind}\">{content}</{prefix}c>");
        if let Some(row) = row {
            let node = &xml.elements[row];
            if xml.source[node.outer.clone()].ends_with("/>") {
                (
                    node.outer.clone(),
                    format!("{}{new_cell}</{}>", node.tag(&[], false), node.name),
                )
            } else {
                let position = xml
                    .children(row, "c")
                    .find(|&index| {
                        xml.elements[index]
                            .attr("r")
                            .and_then(coordinate)
                            .is_some_and(|current| current > location)
                    })
                    .map_or(node.inner.end, |index| xml.elements[index].outer.start);
                (position..position, new_cell)
            }
        } else {
            let row_number = location.0;
            let new_row = format!("<{prefix}row r=\"{row_number}\">{new_cell}</{prefix}row>");
            if xml.source[data_node.outer.clone()].ends_with("/>") {
                (
                    data_node.outer.clone(),
                    format!(
                        "{}{new_row}</{}>",
                        data_node.tag(&[], false),
                        data_node.name
                    ),
                )
            } else {
                let position = xml
                    .children(data, "row")
                    .find(|&index| {
                        xml.elements[index]
                            .attr("r")
                            .and_then(|row| row.parse::<u32>().ok())
                            .is_some_and(|row| row > row_number)
                    })
                    .map_or(data_node.inner.end, |index| xml.elements[index].outer.start);
                (position..position, new_row)
            }
        }
    };
    let edited = replace_ranges(xml.source, vec![(target, replacement)]);
    Xml::parse(&edited)?;
    package.set(&section.id, edited);
    Ok(Change {
        section: section.label.clone(),
        item: address,
        before,
        after,
    })
}

type Coordinates = (u32, u32);
type CellRange = (Coordinates, Coordinates);

fn sheet_range<'a>(reference: &'a str, current: &'a str) -> Option<(String, CellRange)> {
    let (sheet, range) = reference.rsplit_once('!').unwrap_or((current, reference));
    let sheet = if let Some(name) = sheet
        .strip_prefix('\'')
        .and_then(|sheet| sheet.strip_suffix('\''))
    {
        name.replace("''", "'")
    } else {
        sheet.to_owned()
    };
    Some((sheet, parse_range(range)?))
}

pub(super) fn chart_range(reference: &str) -> Result<(String, Vec<String>), String> {
    let (name, (start, end)) = sheet_range(reference, "").ok_or("图表范围无效")?;
    if name.is_empty()
        || (start.0 != end.0 && start.1 != end.1)
        || (end.0 - start.0 + end.1 - start.1) >= 300
    {
        return Err("图表范围必须是至多 300 个单元格的一行或一列".into());
    }
    let mut addresses = Vec::new();
    for row in start.0..=end.0 {
        for column in start.1..=end.1 {
            addresses.push(format!("{}{row}", super::create::column(column as usize)));
        }
    }
    Ok((name, addresses))
}

pub(super) fn refresh_calculation(
    package: &mut Package,
    main: &str,
    sections: &[Section],
    changes: &[Change],
) -> Result<bool, String> {
    use calc::{Input, Scalar};

    let strings = read_strings(package, main)?;
    let mut data = BTreeMap::new();
    for section in sections {
        let xml = package.xml(&section.id)?;
        let mut values = calc::Sheet::new();
        for index in cells(&xml) {
            let node = &xml.elements[index];
            let address = node
                .attr("r")
                .and_then(coordinate)
                .ok_or("Excel 单元格地址无效")?;
            let value = if let Some(formula) = xml.child(index, "f") {
                if xml.elements[formula].attr("t").is_none() {
                    Input::Formula(xml.text(formula)?)
                } else {
                    Input::Unsupported
                }
            } else {
                let value = cell_text(&xml, index, &strings)?;
                match node.attr("t") {
                    None | Some("n") if value.is_empty() => Input::Value(Scalar::Empty),
                    None | Some("n") => value
                        .parse::<f64>()
                        .ok()
                        .filter(|value| value.is_finite())
                        .map_or(Input::Unsupported, |value| {
                            Input::Value(Scalar::Number(value))
                        }),
                    Some("s" | "inlineStr" | "str") => Input::Value(Scalar::Text(value)),
                    Some("b") if value == "0" || value == "1" => {
                        Input::Value(Scalar::Boolean(value == "1"))
                    }
                    _ => Input::Unsupported,
                }
            };
            values.insert(address, value);
        }
        data.insert(section.label.clone(), values);
    }

    let mut changed: BTreeSet<_> = changes
        .iter()
        .filter_map(|change| {
            coordinate(&change.item).map(|location| (change.section.clone(), location))
        })
        .collect();
    let mut workbook = calc::Workbook::new(data, &changed);
    let mut pending = false;
    for section in sections {
        let xml = package.xml(&section.id)?;
        let mut replacements = Vec::new();
        for index in cells(&xml) {
            let Some(formula) = xml.child(index, "f") else {
                continue;
            };
            let node = &xml.elements[index];
            let address = node
                .attr("r")
                .and_then(coordinate)
                .ok_or("Excel 公式缺少地址")?;
            let Some((value, affected)) = workbook.number(&section.label, address) else {
                pending = true;
                continue;
            };
            if !changes.is_empty() && !affected {
                continue;
            }
            changed.insert((section.label.clone(), address));
            let value_node = xml.child(index, "v");
            if matches!(node.attr("t"), None | Some("n")) {
                if let Some(cache) = value_node {
                    let raw = xml.text(cache)?;
                    if raw.parse::<f64>() == Ok(value) {
                        continue;
                    }
                }
            } else {
                replacements.push((
                    node.outer.start..node.inner.start,
                    node.tag(&[("t", "n")], false),
                ));
            }
            let prefix = node.name.strip_suffix('c').unwrap();
            let position = xml.elements[formula].outer.end;
            let target = value_node
                .map(|cache| xml.elements[cache].outer.clone())
                .unwrap_or(position..position);
            replacements.push((target, format!("<{prefix}v>{value}</{prefix}v>")));
        }
        if !replacements.is_empty() {
            let edited = replace_ranges(xml.source, replacements);
            package.set(&section.id, edited);
        }
    }
    if !changes.is_empty() && !pending {
        pending |= refresh_charts(package, main, sections, &changed)?;
    }
    if pending {
        let xml = package.xml(main)?;
        if let Some(index) = xml.child(0, "calcPr") {
            let node = &xml.elements[index];
            let updated = node.tag(&[("fullCalcOnLoad", "1")], true);
            package.set(
                main,
                replace_ranges(xml.source, vec![(node.outer.clone(), updated)]),
            );
        }
    }
    Ok(pending)
}

pub(super) fn numeric_column(
    package: &Package,
    part: &str,
    column: u32,
) -> Result<BTreeMap<u32, String>, String> {
    let xml = package.xml(part)?;
    let mut values = BTreeMap::new();
    for index in cells(&xml) {
        let node = &xml.elements[index];
        if let Some((row, _)) = node
            .attr("r")
            .and_then(coordinate)
            .filter(|&(_, col)| col == column)
        {
            if matches!(node.attr("t"), None | Some("n")) {
                let value = cell_text(&xml, index, &[])?;
                if value.parse::<f64>().is_ok_and(f64::is_finite) {
                    values.insert(row, value);
                }
            }
        }
    }
    Ok(values)
}

fn refresh_charts(
    package: &mut Package,
    main: &str,
    sections: &[Section],
    changed: &BTreeSet<(String, Coordinates)>,
) -> Result<bool, String> {
    let chart_parts: Vec<_> = package
        .parts
        .keys()
        .filter(|part| {
            part.starts_with("xl/charts/") && !part.contains("/_rels/") && part.ends_with(".xml")
        })
        .cloned()
        .collect();
    if chart_parts.is_empty() {
        return Ok(false);
    }
    let strings = read_strings(package, main)?;
    let mut pending = false;
    for part in chart_parts {
        let xml = package.xml(&part)?;
        if xml.named("externalData").next().is_some()
            || xml.named("multiLvlStrRef").next().is_some()
        {
            pending = true;
            continue;
        }
        let mut replacements = Vec::new();
        for (reference, cache) in [("numRef", "numCache"), ("strRef", "strCache")] {
            for index in xml.named(reference) {
                let Some(formula) = xml.child(index, "f") else {
                    pending = true;
                    continue;
                };
                let formula = xml.text(formula)?;
                let Ok((name, addresses)) = chart_range(&formula) else {
                    pending = true;
                    continue;
                };
                if !addresses.iter().any(|address| {
                    coordinate(address)
                        .is_some_and(|location| changed.contains(&(name.clone(), location)))
                }) {
                    continue;
                }
                let Some(cache) = xml.child(index, cache) else {
                    pending = true;
                    continue;
                };
                let Some(section) = sections.iter().find(|section| section.label == name) else {
                    pending = true;
                    continue;
                };
                let sheet = package.xml(&section.id)?;
                let indices: BTreeMap<_, _> = cells(&sheet)
                    .filter_map(|index| Some((sheet.elements[index].attr("r")?, index)))
                    .collect();
                let mut values = Vec::with_capacity(addresses.len());
                for address in &addresses {
                    let Some(index) = indices.get(address.as_str()) else {
                        break;
                    };
                    let value = cell_text(&sheet, *index, &strings)?;
                    if reference == "numRef" && !value.parse::<f64>().is_ok_and(f64::is_finite) {
                        break;
                    }
                    values.push(value);
                }
                if values.len() != addresses.len() {
                    pending = true;
                    continue;
                }
                replacements.push(super::chart::cache_update(&xml, cache, &values));
            }
        }
        if !replacements.is_empty() {
            let edited = replace_ranges(xml.source, replacements);
            package.set(part, edited);
        }
    }
    Ok(pending)
}

pub(super) fn set_number_format(
    package: &mut Package,
    main: &str,
    section: &Section,
    address: &str,
    format: &str,
) -> Result<Change, String> {
    let format_id = match format {
        "General" => "0",
        "0" => "1",
        "0.00" => "2",
        "#,##0" => "3",
        "#,##0.00" => "4",
        "0%" => "9",
        "0.00%" => "10",
        "m/d/yy" => "14",
        _ => return Err("该 Excel 数字格式暂不支持".into()),
    };
    let address = address.trim().to_ascii_uppercase();
    let location = coordinate(&address).ok_or("Excel 单元格地址无效")?;
    let xml = package.xml(&section.id)?;
    if xml.named("sheetProtection").next().is_some()
        || merged_ranges(&xml)
            .iter()
            .any(|range| within(location, *range))
    {
        return Err("Excel 合并区或受保护的单元格暂不可编辑".into());
    }
    let cell = cells(&xml)
        .find(|&index| xml.elements[index].attr("r") == Some(&address))
        .ok_or("数字格式修改需要已有单元格")?;
    let cell = &xml.elements[cell];
    let styles_part = package
        .related(main, "styles")?
        .ok_or("Excel 工作簿缺少样式部件")?;
    let styles = package.xml(&styles_part)?;
    let xfs = styles
        .child(0, "cellXfs")
        .ok_or("Excel 工作簿缺少单元格样式")?;
    let formats: Vec<_> = styles.children(xfs, "xf").collect();
    let previous = cell
        .attr("s")
        .unwrap_or("0")
        .parse::<usize>()
        .ok()
        .and_then(|index| formats.get(index))
        .ok_or("Excel 样式索引无效")?;
    let old = &styles.elements[*previous];
    let before = old.attr("numFmtId").unwrap_or("0").to_owned();
    if before == format_id {
        return Err("所选单元格已使用此数字格式".into());
    }
    let source = styles.source;
    let empty = source[old.outer.clone()].ends_with("/>");
    let mut updated = old.tag(
        &[("numFmtId", format_id), ("applyNumberFormat", "1")],
        empty,
    );
    if !empty {
        updated.push_str(&source[old.inner.start..old.outer.end]);
    }
    let xfs = &styles.elements[xfs];
    let count = (formats.len() + 1).to_string();
    let next_style = formats.len().to_string();
    let new_formats = format!(
        "{}{}{updated}</{}>",
        xfs.tag(&[("count", &count)], false),
        &source[xfs.inner.clone()],
        xfs.name
    );
    let edited_styles = replace_ranges(source, vec![(xfs.outer.clone(), new_formats)]);
    let mut updated_cell = cell.tag(
        &[("s", &next_style)],
        xml.source[cell.outer.clone()].ends_with("/>"),
    );
    updated_cell.push_str(&xml.source[cell.inner.start..cell.outer.end]);
    let edited_sheet = replace_ranges(xml.source, vec![(cell.outer.clone(), updated_cell)]);
    package.set(styles_part, edited_styles);
    package.set(&section.id, edited_sheet);
    Ok(Change {
        section: section.label.clone(),
        item: address,
        before: format!("Excel 格式 {before}"),
        after: format.to_owned(),
    })
}
