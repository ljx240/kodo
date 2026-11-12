use super::sheet::{self, cell_parts};
use super::xml::{escape, validate_text};
use super::{
    chart, CellValue, ChartKind, Format, Package, Slide, SlideChart, SlideLayout, SlideMetric,
};
use serde::{Deserialize, Deserializer};
use std::collections::BTreeSet;

const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const X: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const MIME: &str = "application/vnd.openxmlformats-officedocument";

pub(super) fn build(format: Format, content: &str) -> Result<(Package, bool), String> {
    let mut package = Package::default();
    let ((main, overrides), pending) = match format {
        Format::Docx => (word(&mut package, content)?, false),
        Format::Xlsx => workbook(&mut package, content)?,
        Format::Pptx => (presentation(&mut package, content)?, false),
    };
    package.set(
        "_rels/.rels",
        relationships(&[("officeDocument", main.to_owned())]),
    );
    let entries: String = std::iter::once((main.to_owned(), format.content_type().to_owned()))
        .chain(overrides)
        .map(|(part, kind)| format!(r#"<Override PartName="/{part}" ContentType="{kind}"/>"#))
        .collect();
    package.set("[Content_Types].xml", format!(r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>{entries}</Types>"#));
    Ok((package, pending))
}

type Parts = (&'static str, Vec<(String, String)>);

fn relationships(entries: &[(&str, String)]) -> String {
    let children: String = entries
        .iter()
        .enumerate()
        .map(|(index, (kind, target))| {
            format!(
                r#"<Relationship Id="rId{}" Type="{REL}/{kind}" Target="{}"/>"#,
                index + 1,
                escape(target)
            )
        })
        .collect();
    format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{children}</Relationships>"#
    )
}

fn word(package: &mut Package, content: &str) -> Result<Parts, String> {
    if content.trim().is_empty() {
        return Err("Word 文档需要正文".into());
    }
    let mut body = String::new();
    let mut lines = content.lines().map(str::trim).peekable();
    while let Some(line) = lines.next() {
        if let Some(first) = word_table_row(line) {
            let mut rows = vec![first];
            while let Some(row) = lines.peek().and_then(|line| word_table_row(line)) {
                lines.next();
                rows.push(row);
            }
            rows.retain(|row| {
                !row.iter()
                    .all(|cell| !cell.is_empty() && cell.chars().all(|c| c == '-' || c == ':'))
            });
            if !rows.is_empty() {
                word_table(&mut body, &rows)?;
            }
            continue;
        }
        let (style, text) = if let Some(text) = line.strip_prefix("# ") {
            ("Title", text)
        } else if let Some(text) = line.strip_prefix("## ") {
            ("Heading1", text)
        } else if let Some(text) = line.strip_prefix("### ") {
            ("Heading2", text)
        } else if let Some(text) = line.strip_prefix("- ") {
            body.push_str(&format!(r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#, escape(text)));
            continue;
        } else {
            ("Normal", line)
        };
        if !text.is_empty() {
            body.push_str(&word_paragraph(text, style));
        }
    }
    package.set("word/document.xml", format!(r#"<w:document xmlns:w="{W}" xmlns:r="{REL}"><w:body>{body}<w:sectPr><w:footerReference w:type="default" r:id="rId2"/><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1134" w:right="1134" w:bottom="1134" w:left="1134" w:header="567" w:footer="567" w:gutter="0"/></w:sectPr></w:body></w:document>"#));
    let mut styles = format!(
        r#"<w:styles xmlns:w="{W}"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Aptos" w:hAnsi="Aptos" w:eastAsia="Microsoft YaHei"/><w:sz w:val="22"/></w:rPr></w:rPrDefault></w:docDefaults>"#
    );
    styles.push_str(r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:pPr><w:widowControl/><w:spacing w:after="130" w:line="300" w:lineRule="auto"/></w:pPr></w:style>"#);
    for (name, size, outline) in [
        ("Title", 46, ""),
        ("Heading1", 31, r#"<w:outlineLvl w:val="0"/>"#),
        ("Heading2", 26, r#"<w:outlineLvl w:val="1"/>"#),
    ] {
        styles.push_str(&format!(r#"<w:style w:type="paragraph" w:styleId="{name}"><w:name w:val="{name}"/><w:basedOn w:val="Normal"/><w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before="200" w:after="150"/>{outline}</w:pPr><w:rPr><w:b/><w:color w:val="000000"/><w:sz w:val="{size}"/></w:rPr></w:style>"#));
    }
    for (name, color, bold) in [
        ("OfficeTableText", "000000", ""),
        ("OfficeTableHead", "FFFFFF", "<w:b/>"),
    ] {
        styles.push_str(&format!(r#"<w:style w:type="paragraph" w:styleId="{name}"><w:name w:val="{name}"/><w:basedOn w:val="Normal"/><w:pPr><w:spacing w:before="0" w:after="0" w:line="260" w:lineRule="auto"/></w:pPr><w:rPr>{bold}<w:color w:val="{color}"/><w:sz w:val="21"/></w:rPr></w:style>"#));
    }
    styles.push_str(r#"<w:style w:type="table" w:styleId="OfficeTable"><w:name w:val="Office Table"/><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4" w:color="D9D9D9"/><w:left w:val="single" w:sz="4" w:color="D9D9D9"/><w:bottom w:val="single" w:sz="4" w:color="D9D9D9"/><w:right w:val="single" w:sz="4" w:color="D9D9D9"/><w:insideH w:val="single" w:sz="4" w:color="D9D9D9"/><w:insideV w:val="single" w:sz="4" w:color="D9D9D9"/></w:tblBorders></w:tblPr></w:style></w:styles>"#);
    package.set("word/styles.xml", styles);
    package.set("word/footer1.xml", format!(r#"<w:ftr xmlns:w="{W}"><w:p><w:pPr><w:jc w:val="right"/></w:pPr><w:fldSimple w:instr=" PAGE "><w:r><w:t>1</w:t></w:r></w:fldSimple></w:p></w:ftr>"#));
    package.set("word/numbering.xml", format!(r#"<w:numbering xmlns:w="{W}"><w:abstractNum w:abstractNumId="0"><w:multiLevelType w:val="singleLevel"/><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="bullet"/><w:lvlText w:val="•"/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="420" w:hanging="240"/></w:pPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#));
    package.set(
        "word/_rels/document.xml.rels",
        relationships(&[
            ("styles", "styles.xml".to_owned()),
            ("footer", "footer1.xml".to_owned()),
            ("numbering", "numbering.xml".to_owned()),
        ]),
    );
    Ok((
        "word/document.xml",
        [
            ("styles.xml", "styles"),
            ("footer1.xml", "footer"),
            ("numbering.xml", "numbering"),
        ]
        .into_iter()
        .map(|(file, part)| {
            (
                format!("word/{file}"),
                format!("{MIME}.wordprocessingml.{part}+xml"),
            )
        })
        .collect(),
    ))
}

fn word_paragraph(text: &str, style: &str) -> String {
    format!(
        r#"<w:p><w:pPr><w:pStyle w:val="{style}"/></w:pPr><w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
        escape(text)
    )
}

fn word_table_row(line: &str) -> Option<Vec<&str>> {
    Some(
        line.strip_prefix('|')?
            .strip_suffix('|')?
            .split('|')
            .map(str::trim)
            .collect(),
    )
}

fn word_table(output: &mut String, rows: &[Vec<&str>]) -> Result<(), String> {
    const WIDTH: usize = 9_280;
    let columns = rows.iter().map(Vec::len).max().unwrap();
    if columns > 32 {
        return Err("Word 表格最多支持 32 列".into());
    }
    let minimum = (WIDTH / columns).min(1_160);
    let weights: Vec<_> = (0..columns)
        .map(|index| {
            rows.iter()
                .filter_map(|row| row.get(index))
                .map(|text| text.chars().count())
                .max()
                .unwrap_or(0)
                .clamp(6, 34)
        })
        .collect();
    let sum: usize = weights.iter().sum();
    let mut widths: Vec<_> = weights
        .iter()
        .map(|weight| minimum + (WIDTH - minimum * columns) * weight / sum)
        .collect();
    let remainder = WIDTH - widths.iter().sum::<usize>();
    *widths.last_mut().unwrap() += remainder;
    output.push_str(&format!(r#"<w:tbl><w:tblPr><w:tblStyle w:val="OfficeTable"/><w:tblW w:w="{WIDTH}" w:type="dxa"/><w:tblLayout w:type="fixed"/><w:tblCellMar><w:top w:w="65" w:type="dxa"/><w:left w:w="125" w:type="dxa"/><w:bottom w:w="65" w:type="dxa"/><w:right w:w="125" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblGrid>"#));
    for width in &widths {
        output.push_str(&format!(r#"<w:gridCol w:w="{width}"/>"#));
    }
    output.push_str("</w:tblGrid>");
    for (index, row) in rows.iter().enumerate() {
        let (style, fill, header) = match index {
            0 => ("OfficeTableHead", "183451", "<w:tblHeader/>"),
            n if n % 2 == 0 => ("OfficeTableText", "F4F7FA", ""),
            _ => ("OfficeTableText", "FFFFFF", ""),
        };
        output.push_str(&format!("<w:tr><w:trPr><w:cantSplit/>{header}</w:trPr>"));
        for (column, width) in widths.iter().enumerate() {
            let text = row.get(column).copied().unwrap_or("");
            output.push_str(&format!(r#"<w:tc><w:tcPr><w:tcW w:w="{width}" w:type="dxa"/><w:shd w:val="clear" w:color="auto" w:fill="{fill}"/><w:vAlign w:val="center"/></w:tcPr>{}</w:tc>"#, word_paragraph(text, style)));
        }
        output.push_str("</w:tr>");
    }
    output.push_str("</w:tbl>");
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Workbook {
    sheets: Vec<Sheet>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Sheet {
    name: String,
    #[serde(deserialize_with = "decode_rows")]
    rows: Vec<Vec<CellValue>>,
    #[serde(default)]
    column_widths: Vec<u16>,
    chart: Option<WorksheetChart>,
}

fn decode_rows<'de, D: Deserializer<'de>>(decoder: D) -> Result<Vec<Vec<CellValue>>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum InputCell {
        Typed(CellValue),
        Text(String),
        Number(serde_json::Number),
        Boolean(bool),
        Empty(()),
    }

    let input = Vec::<Vec<InputCell>>::deserialize(decoder)?;
    Ok(input
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|cell| match cell {
                    InputCell::Typed(value) => value,
                    InputCell::Text(value) => CellValue::Text(value),
                    InputCell::Number(value) => CellValue::Number(value.to_string()),
                    InputCell::Boolean(value) => CellValue::Boolean(value),
                    InputCell::Empty(()) => CellValue::Blank,
                })
                .collect()
        })
        .collect())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorksheetChart {
    kind: ChartKind,
    title: String,
    category_column: usize,
    value_column: usize,
    #[serde(default)]
    below_table: bool,
}

pub(super) fn column(mut number: usize) -> String {
    let mut letters = Vec::new();
    while number > 0 {
        number -= 1;
        letters.push(b'A' + (number % 26) as u8);
        number /= 26;
    }
    letters.reverse();
    String::from_utf8(letters).unwrap()
}

fn workbook(package: &mut Package, content: &str) -> Result<(Parts, bool), String> {
    let input: Workbook =
        serde_json::from_str(content).map_err(|error| format!("Excel 内容格式错误：{error}"))?;
    if input.sheets.is_empty() || input.sheets.len() > 32 {
        return Err("Excel 工作簿需要 1 至 32 张工作表".into());
    }
    let mut names = BTreeSet::new();
    let mut sheet_index = String::new();
    let mut rels = Vec::new();
    let mut overrides = Vec::new();
    let mut charts = Vec::new();
    for (index, sheet) in input.sheets.iter().enumerate() {
        let name = &sheet.name;
        if name.trim().is_empty()
            || name.chars().count() > 31
            || name.contains([':', '/', '\\', '?', '*', '[', ']'])
            || name.starts_with('\'')
            || name.ends_with('\'')
            || !names.insert(name.to_lowercase())
        {
            return Err("Excel 工作表名无效或重复".into());
        }
        validate_text(name)?;
        let n = index + 1;
        sheet_index.push_str(&format!(
            r#"<sheet name="{}" sheetId="{n}" r:id="rId{n}"/>"#,
            escape(name)
        ));
        let file = format!("worksheets/sheet{n}.xml");
        rels.push(("worksheet", file.clone()));
        let mut rows = String::new();
        let mut columns = 0;
        for (row_index, row) in sheet.rows.iter().enumerate() {
            let r = row_index + 1;
            if row.len() > 16_384 || r > 1_048_576 {
                return Err("Excel 内容超出最大行列范围".into());
            }
            columns = columns.max(row.len());
            rows.push_str(&format!(r#"<row r="{r}">"#));
            for (col, value) in row.iter().enumerate() {
                let (kind, cell, _) = cell_parts(value, "")?;
                let address = format!("{}{r}", column(col + 1));
                let style = if r == 1 { r#" s="1""# } else { "" };
                rows.push_str(&format!(r#"<c r="{address}" t="{kind}"{style}>{cell}</c>"#));
            }
            rows.push_str("</row>");
        }
        let filter = if columns > 0 && sheet.rows.len() > 1 {
            format!(
                r#"<autoFilter ref="A1:{}{}"/>"#,
                column(columns),
                sheet.rows.len()
            )
        } else {
            String::new()
        };
        if sheet.column_widths.len() > columns
            || sheet
                .column_widths
                .iter()
                .any(|width| !(8..=90).contains(width))
        {
            return Err("Excel 列宽需要与表格对应，且在 8 至 90 之间".into());
        }
        let widths = if columns > 0 {
            let mut children = String::new();
            for (index, width) in sheet.column_widths.iter().enumerate() {
                let column = index + 1;
                children.push_str(&format!(
                    r#"<col min="{column}" max="{column}" width="{width}" customWidth="1"/>"#
                ));
            }
            if sheet.column_widths.len() < columns {
                let start = sheet.column_widths.len() + 1;
                children.push_str(&format!(
                    r#"<col min="{start}" max="{columns}" width="19" customWidth="1"/>"#
                ));
            }
            format!("<cols>{children}</cols>")
        } else {
            String::new()
        };
        let drawing = if let Some(spec) = &sheet.chart {
            charts.push((n, sheet, spec, columns));
            r#"<drawing r:id="rId1"/>"#
        } else {
            ""
        };
        let part = format!("xl/{file}");
        package.set(&part, format!(r#"<worksheet xmlns="{X}" xmlns:r="{REL}"><sheetPr><pageSetUpPr fitToPage="1"/></sheetPr><sheetViews><sheetView workbookViewId="0"><pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/><selection pane="bottomLeft" activeCell="A2" sqref="A2"/></sheetView></sheetViews>{widths}<sheetData>{rows}</sheetData>{filter}<pageSetup orientation="landscape" fitToWidth="1" fitToHeight="0"/>{drawing}</worksheet>"#));
        overrides.push((part, format!("{MIME}.spreadsheetml.worksheet+xml")));
    }
    rels.push(("styles", "styles.xml".to_owned()));
    package.set("xl/workbook.xml", format!(r#"<workbook xmlns="{X}" xmlns:r="{REL}"><bookViews><workbookView/></bookViews><sheets>{sheet_index}</sheets><calcPr calcId="191029"/></workbook>"#));
    package.set("xl/_rels/workbook.xml.rels", relationships(&rels));
    package.set("xl/styles.xml", format!(r#"<styleSheet xmlns="{X}"><fonts count="2"><font><sz val="11"/><name val="Aptos"/></font><font><b/><sz val="11"/><color rgb="FFFFFFFF"/><name val="Aptos"/></font></fonts><fills count="3"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FF183451"/><bgColor indexed="64"/></patternFill></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="0" fontId="1" fillId="2" borderId="0" xfId="0" applyFill="1" applyFont="1"/></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>"#));
    overrides.push((
        "xl/styles.xml".to_owned(),
        format!("{MIME}.spreadsheetml.styles+xml"),
    ));
    let main = "xl/workbook.xml";
    let sections = sheet::sections(package, main)?;
    let pending = sheet::refresh_calculation(package, main, &sections, &[])?;
    for (n, sheet, spec, columns) in charts {
        worksheet_chart(package, &mut overrides, n, sheet, spec, columns)?;
    }
    Ok(((main, overrides), pending))
}

fn worksheet_chart(
    package: &mut Package,
    overrides: &mut Vec<(String, String)>,
    n: usize,
    sheet: &Sheet,
    spec: &WorksheetChart,
    columns: usize,
) -> Result<(), String> {
    if !(1..=columns).contains(&spec.category_column)
        || !(1..=columns).contains(&spec.value_column)
        || columns > 16_376
    {
        return Err("图表列必须是有效的 1 基序号".into());
    }
    let mut categories = Vec::new();
    let mut values = Vec::new();
    let part = format!("xl/worksheets/sheet{n}.xml");
    let numbers = sheet::numeric_column(package, &part, spec.value_column as u32)?;
    for (index, row) in sheet.rows.iter().enumerate().skip(1) {
        match row.get(spec.category_column - 1) {
            Some(CellValue::Text(value) | CellValue::Number(value)) => {
                categories.push(value.clone())
            }
            _ => return Err("图表分类列需要文本或数字".into()),
        }
        values.push(
            numbers
                .get(&(index as u32 + 1))
                .ok_or("图表数值列需要数值或已复算的公式")?
                .clone(),
        );
    }
    let last = sheet.rows.len();
    let name = sheet.name.replace('\'', "''");
    let category = column(spec.category_column);
    let value = column(spec.value_column);
    let category_ref = format!("'{name}'!${category}$2:${category}${last}");
    let value_ref = format!("'{name}'!${value}$2:${value}${last}");
    let (left, top, right, bottom) = if spec.below_table {
        (0, last + 1, columns.max(6), last + 16)
    } else {
        (columns, 1, columns + 7, 17)
    };
    let spec = SlideChart {
        kind: spec.kind,
        title: spec.title.clone(),
        categories,
        values,
    };
    let chart = format!("xl/charts/chart{n}.xml");
    package.set(&chart, chart::xml(&spec, &category_ref, &value_ref, false)?);
    let drawing = format!("xl/drawings/drawing{n}.xml");
    package.set(&drawing, format!(r#"<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="{A}" xmlns:r="{REL}"><xdr:twoCellAnchor><xdr:from><xdr:col>{left}</xdr:col><xdr:colOff>152400</xdr:colOff><xdr:row>{top}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>{right}</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>{bottom}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to><xdr:graphicFrame macro=""><xdr:nvGraphicFramePr><xdr:cNvPr id="2" name="图表"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/></xdr:xfrm>{}</xdr:graphicFrame><xdr:clientData/></xdr:twoCellAnchor></xdr:wsDr>"#, chart_frame()));
    package.set(
        format!("xl/worksheets/_rels/sheet{n}.xml.rels"),
        relationships(&[("drawing", format!("../drawings/drawing{n}.xml"))]),
    );
    package.set(
        format!("xl/drawings/_rels/drawing{n}.xml.rels"),
        relationships(&[("chart", format!("../charts/chart{n}.xml"))]),
    );
    overrides.extend([
        (drawing, format!("{MIME}.drawing+xml")),
        (chart, format!("{MIME}.drawingml.chart+xml")),
    ]);
    Ok(())
}

fn chart_frame() -> &'static str {
    r#"<a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="rId1"/></a:graphicData></a:graphic>"#
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Presentation {
    slides: Vec<Slide>,
}

const GROUP: &str = r#"<p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>"#;
const COLORS: &str = r#"<p:clrMap accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" bg1="lt1" bg2="lt2" folHlink="folHlink" hlink="hlink" tx1="dk1" tx2="dk2"/>"#;
const OVERRIDE: &str = "<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>";

fn shape(
    id: u32,
    title: &str,
    lines: &[String],
    bounds: [u32; 4],
    size: u32,
    color: &str,
    bold: bool,
) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let [x, y, width, height] = bounds;
    let bold = u8::from(bold);
    let body: String = lines.iter().map(|line| format!(r#"<a:p><a:pPr><a:spcAft><a:spcPts val="1150"/></a:spcAft></a:pPr><a:r><a:rPr lang="zh-CN" sz="{size}" b="{bold}"><a:solidFill><a:srgbClr val="{color}"/></a:solidFill><a:latin typeface="Aptos"/><a:ea typeface="Microsoft YaHei"/></a:rPr><a:t>{}</a:t></a:r></a:p>"#, escape(line))).collect();
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="{title}"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{width}" cy="{height}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:noFill/></p:spPr><p:txBody><a:bodyPr lIns="0" tIns="0" rIns="0" bIns="0" wrap="square"><a:normAutofit/></a:bodyPr><a:lstStyle/>{body}</p:txBody></p:sp>"#
    )
}

fn metric_card(
    id: u32,
    x: u32,
    width: u32,
    metric: &SlideMetric,
    ink: &str,
) -> Result<String, String> {
    for (value, limit) in [
        (&metric.label, 24),
        (&metric.value, 16),
        (&metric.detail, 55),
    ] {
        if value.chars().count() > limit {
            return Err(format!("指标字段最多 {limit} 字符"));
        }
        validate_text(value)?;
    }
    if metric.label.trim().is_empty() || metric.value.trim().is_empty() {
        return Err("指标需要名称和数值".into());
    }
    let mut card = format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="指标背景"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="{x}" y="2100000"/><a:ext cx="{width}" cy="2670000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="EDF4F8"/></a:solidFill><a:ln><a:noFill/></a:ln></p:spPr></p:sp>"#
    );
    let x = x + 255_000;
    let width = width - 510_000;
    card.push_str(&shape(
        id + 1,
        "指标名称",
        std::slice::from_ref(&metric.label),
        [x, 2_450_000, width, 480_000],
        1700,
        ink,
        false,
    ));
    card.push_str(&shape(
        id + 2,
        "指标数值",
        std::slice::from_ref(&metric.value),
        [x, 3_050_000, width, 900_000],
        4200,
        ink,
        true,
    ));
    if !metric.detail.is_empty() {
        card.push_str(&shape(
            id + 3,
            "指标口径",
            std::slice::from_ref(&metric.detail),
            [x, 4_000_000, width, 615_000],
            1450,
            "425970",
            false,
        ));
    }
    Ok(card)
}

fn slide_text(slide: &Slide) -> Result<(&'static str, String), String> {
    use SlideLayout::*;

    const LEFT: u32 = 830_000;
    const WIDTH: u32 = 10_520_000;
    const INK: &str = "183451";
    const BODY: &str = "24394D";

    if slide.layout != Comparison && !slide.secondary.is_empty() {
        return Err("secondary 仅用于双栏对比版式".into());
    }
    if slide.layout != Metrics && !slide.metrics.is_empty() {
        return Err("metrics 仅用于指标版式".into());
    }
    if slide.chart.is_some() && !matches!(slide.layout, Body | Trend) {
        return Err("原生图表可放在正文或趋势版式".into());
    }
    if slide.layout == Trend && slide.chart.is_none() {
        return Err("趋势页需要原生图表数据".into());
    }
    let title = std::slice::from_ref(&slide.title);
    if slide.layout == Cover {
        if slide.body.len() > 3 {
            return Err("封面正文最多 3 项".into());
        }
        let title = shape(
            2,
            "标题",
            title,
            [LEFT, 1_920_000, WIDTH, 1_880_000],
            4600,
            "FFFFFF",
            true,
        );
        let subtitle = shape(
            3,
            "副标题",
            &slide.body,
            [LEFT, 4_020_000, WIDTH, 1_470_000],
            2350,
            "D6E4EE",
            false,
        );
        return Ok((INK, title + &subtitle));
    }

    let mut text = shape(
        2,
        "标题",
        title,
        [LEFT, 640_000, WIDTH, 1_350_000],
        3400,
        INK,
        true,
    );
    match slide.layout {
        Comparison => {
            if !(1..=6).contains(&slide.body.len()) || !(1..=6).contains(&slide.secondary.len()) {
                return Err("对比页的每栏需要 1 至 6 项".into());
            }
            text.push_str(&shape(
                3,
                "左栏",
                &slide.body,
                [LEFT, 2_020_000, 4_940_000, 3_810_000],
                2050,
                BODY,
                false,
            ));
            text.push_str(&shape(
                5,
                "右栏",
                &slide.secondary,
                [6_410_000, 2_020_000, 4_940_000, 3_810_000],
                2050,
                BODY,
                false,
            ));
        }
        Metrics => {
            if !(2..=4).contains(&slide.metrics.len()) || slide.body.len() > 2 {
                return Err("指标页需要 2 至 4 个指标，解读最多 2 项".into());
            }
            let count = slide.metrics.len() as u32;
            let gap = 240_000;
            let width = (WIDTH - (count - 1) * gap) / count;
            for (index, metric) in slide.metrics.iter().enumerate() {
                let index = index as u32;
                text.push_str(&metric_card(
                    10 + index * 4,
                    LEFT + index * (width + gap),
                    width,
                    metric,
                    INK,
                )?);
            }
            text.push_str(&shape(
                3,
                "指标解读",
                &slide.body,
                [LEFT, 5_090_000, WIDTH, 1_130_000],
                1800,
                BODY,
                false,
            ));
        }
        Conclusion => {
            let (statement, evidence) = slide.body.split_first().ok_or("结论页需要一句主要结论")?;
            text.push_str(&shape(
                3,
                "主要结论",
                std::slice::from_ref(statement),
                [LEFT, 2_020_000, WIDTH, 1_430_000],
                2950,
                INK,
                false,
            ));
            text.push_str(&shape(
                5,
                "依据",
                evidence,
                [LEFT, 3_720_000, WIDTH, 2_350_000],
                2050,
                BODY,
                false,
            ));
        }
        Body | Trend | Action => {
            let height = if slide.chart.is_some() {
                900_000
            } else {
                3_810_000
            };
            let name = if slide.layout == Action {
                "行动项"
            } else {
                "正文"
            };
            text.push_str(&shape(
                3,
                name,
                &slide.body,
                [LEFT, 2_020_000, WIDTH, height],
                2050,
                BODY,
                false,
            ));
        }
        Cover => unreachable!("cover layout is rendered above"),
    }
    Ok(("FFFFFF", text))
}

fn presentation(package: &mut Package, content: &str) -> Result<Parts, String> {
    let input: Presentation =
        serde_json::from_str(content).map_err(|error| format!("PPT 内容格式错误：{error}"))?;
    if input.slides.is_empty() || input.slides.len() > 80 {
        return Err("PowerPoint 需要 1 至 80 页".into());
    }
    let mut rels = vec![
        ("slideMaster", "slideMasters/slideMaster1.xml".to_owned()),
        ("notesMaster", "notesMasters/notesMaster1.xml".to_owned()),
    ];
    let mut slides = String::new();
    let mut overrides = vec![
        (
            "ppt/slideMasters/slideMaster1.xml".to_owned(),
            format!("{MIME}.presentationml.slideMaster+xml"),
        ),
        (
            "ppt/slideLayouts/slideLayout1.xml".to_owned(),
            format!("{MIME}.presentationml.slideLayout+xml"),
        ),
        (
            "ppt/notesMasters/notesMaster1.xml".to_owned(),
            format!("{MIME}.presentationml.notesMaster+xml"),
        ),
        (
            "ppt/theme/theme1.xml".to_owned(),
            format!("{MIME}.theme+xml"),
        ),
    ];
    for (index, slide) in input.slides.into_iter().enumerate() {
        if slide.title.trim().is_empty()
            || slide.title.chars().count() > 110
            || slide.body.len() > 10
            || slide
                .body
                .iter()
                .chain(&slide.secondary)
                .any(|line| line.chars().count() > 180)
        {
            return Err("每页需要标题，正文最多 10 项，请拆分内容过多的页面".into());
        }
        validate_text(&slide.title)?;
        validate_text(&slide.notes)?;
        for text in slide.body.iter().chain(&slide.secondary) {
            validate_text(text)?;
        }
        let n = index + 1;
        let id = index + 256;
        let rid = index + 3;
        slides.push_str(&format!(r#"<p:sldId id="{id}" r:id="rId{rid}"/>"#));
        rels.push(("slide", format!("slides/slide{n}.xml")));
        let (background, slide_text) = slide_text(&slide)?;
        let mut slide_rels = vec![("slideLayout", "../slideLayouts/slideLayout1.xml".to_owned())];
        let graphic = if let Some(chart) = &slide.chart {
            let index = slide_rels.len() + 1;
            slide_rels.push(("chart", format!("../charts/chart{n}.xml")));
            let chart_part = format!("ppt/charts/chart{n}.xml");
            let embedded = format!("ppt/embeddings/data{n}.xlsx");
            chart::check_data(&chart.categories, &chart.values)?;
            let mut rows = vec![vec![
                CellValue::Text("分类".into()),
                CellValue::Text(chart.title.clone()),
            ]];
            rows.extend(
                chart
                    .categories
                    .iter()
                    .zip(&chart.values)
                    .map(|(category, value)| {
                        vec![
                            CellValue::Text(category.clone()),
                            CellValue::Number(value.clone()),
                        ]
                    }),
            );
            let sheet = serde_json::json!({"sheets": [{"name":"数据", "rows":rows}]}).to_string();
            let workbook = super::Draft::create(Format::Xlsx, &sheet)?;
            let last = rows.len();
            package.set(&embedded, workbook.bytes);
            package.set(
                &chart_part,
                chart::xml(
                    chart,
                    &format!("数据!$A$2:$A${last}"),
                    &format!("数据!$B$2:$B${last}"),
                    true,
                )?,
            );
            package.set(
                format!("ppt/charts/_rels/chart{n}.xml.rels"),
                relationships(&[("package", format!("../embeddings/data{n}.xlsx"))]),
            );
            overrides.extend([
                (chart_part, format!("{MIME}.drawingml.chart+xml")),
                (embedded, format!("{MIME}.spreadsheetml.sheet")),
            ]);
            let frame = chart_frame().replace("rId1", &format!("rId{index}"));
            format!(
                r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="4" name="图表"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="830000" y="2980000"/><a:ext cx="10520000" cy="3420000"/></p:xfrm>{frame}</p:graphicFrame>"#
            )
        } else {
            String::new()
        };
        let part = format!("ppt/slides/slide{n}.xml");
        package.set(&part, format!(r#"<p:sld xmlns:a="{A}" xmlns:p="{P}" xmlns:r="{REL}"><p:cSld><p:bg><p:bgPr><a:solidFill><a:srgbClr val="{background}"/></a:solidFill><a:effectLst/></p:bgPr></p:bg><p:spTree>{GROUP}{slide_text}{graphic}</p:spTree></p:cSld>{OVERRIDE}</p:sld>"#));
        overrides.push((part, format!("{MIME}.presentationml.slide+xml")));
        if !slide.notes.trim().is_empty() {
            let notes = escape(&slide.notes);
            let notes_part = format!("ppt/notesSlides/notesSlide{n}.xml");
            slide_rels.push(("notesSlide", format!("../notesSlides/notesSlide{n}.xml")));
            package.set(&notes_part, format!(r#"<p:notes xmlns:a="{A}" xmlns:p="{P}"><p:cSld><p:spTree>{GROUP}<p:sp><p:nvSpPr><p:cNvPr id="2" name="演讲者备注"/><p:cNvSpPr/><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>{notes}</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld>{OVERRIDE}</p:notes>"#));
            package.set(
                format!("ppt/notesSlides/_rels/notesSlide{n}.xml.rels"),
                relationships(&[
                    ("notesMaster", "../notesMasters/notesMaster1.xml".to_owned()),
                    ("slide", format!("../slides/slide{n}.xml")),
                ]),
            );
            overrides.push((notes_part, format!("{MIME}.presentationml.notesSlide+xml")));
        }
        package.set(
            format!("ppt/slides/_rels/slide{n}.xml.rels"),
            relationships(&slide_rels),
        );
    }
    package.set("ppt/presentation.xml", format!(r#"<p:presentation xmlns:a="{A}" xmlns:p="{P}" xmlns:r="{REL}"><p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst><p:notesMasterIdLst><p:notesMasterId r:id="rId2"/></p:notesMasterIdLst><p:sldIdLst>{slides}</p:sldIdLst><p:sldSz cx="12192000" cy="6858000" type="screen16x9"/><p:notesSz cx="6858000" cy="9144000"/></p:presentation>"#));
    package.set("ppt/_rels/presentation.xml.rels", relationships(&rels));
    package.set("ppt/slideMasters/slideMaster1.xml", format!(r#"<p:sldMaster xmlns:a="{A}" xmlns:p="{P}" xmlns:r="{REL}"><p:cSld><p:spTree>{GROUP}</p:spTree></p:cSld>{COLORS}<p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/></p:sldLayoutIdLst><p:txStyles><p:titleStyle/><p:bodyStyle/><p:otherStyle/></p:txStyles></p:sldMaster>"#));
    package.set(
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        relationships(&[
            ("slideLayout", "../slideLayouts/slideLayout1.xml".to_owned()),
            ("theme", "../theme/theme1.xml".to_owned()),
        ]),
    );
    package.set("ppt/slideLayouts/slideLayout1.xml", format!(r#"<p:sldLayout xmlns:a="{A}" xmlns:p="{P}" type="blank" preserve="1"><p:cSld name="空白"><p:spTree>{GROUP}</p:spTree></p:cSld>{OVERRIDE}</p:sldLayout>"#));
    package.set(
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
        relationships(&[("slideMaster", "../slideMasters/slideMaster1.xml".to_owned())]),
    );
    package.set("ppt/notesMasters/notesMaster1.xml", format!(r#"<p:notesMaster xmlns:a="{A}" xmlns:p="{P}"><p:cSld><p:spTree>{GROUP}</p:spTree></p:cSld>{COLORS}<p:notesStyle/></p:notesMaster>"#));
    package.set(
        "ppt/notesMasters/_rels/notesMaster1.xml.rels",
        relationships(&[("theme", "../theme/theme1.xml".to_owned())]),
    );
    package.set("ppt/theme/theme1.xml", theme());
    Ok(("ppt/presentation.xml", overrides))
}

fn theme() -> String {
    let colors: String = [
        ("dk1", "182B3E"),
        ("lt1", "FFFFFF"),
        ("dk2", "24394D"),
        ("lt2", "F1F5F8"),
        ("accent1", "245785"),
        ("accent2", "237B76"),
        ("accent3", "A66528"),
        ("accent4", "655A84"),
        ("accent5", "47718C"),
        ("accent6", "60734A"),
        ("hlink", "245785"),
        ("folHlink", "655A84"),
    ]
    .iter()
    .map(|(name, value)| format!(r#"<a:{name}><a:srgbClr val="{value}"/></a:{name}>"#))
    .collect();
    let fill = r#"<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>"#;
    let fills = fill.repeat(3);
    let lines: String = [9525, 19050, 28575]
        .iter()
        .map(|width| format!(r#"<a:ln w="{width}">{fill}<a:prstDash val="solid"/></a:ln>"#))
        .collect();
    let effects = "<a:effectStyle><a:effectLst/></a:effectStyle>".repeat(3);
    format!(
        r#"<a:theme xmlns:a="{A}" name="Kodo"><a:themeElements><a:clrScheme name="Kodo">{colors}</a:clrScheme><a:fontScheme name="Kodo"><a:majorFont><a:latin typeface="Aptos Display"/><a:ea typeface="Microsoft YaHei"/><a:cs typeface=""/></a:majorFont><a:minorFont><a:latin typeface="Aptos"/><a:ea typeface="Microsoft YaHei"/><a:cs typeface=""/></a:minorFont></a:fontScheme><a:fmtScheme name="Kodo"><a:fillStyleLst>{fills}</a:fillStyleLst><a:lnStyleLst>{lines}</a:lnStyleLst><a:effectStyleLst>{effects}</a:effectStyleLst><a:bgFillStyleLst>{fills}</a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>"#
    )
}
