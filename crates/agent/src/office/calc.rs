use super::{sheet_range, within, CellRange, Coordinates};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
pub(super) enum Scalar {
    Empty,
    Number(f64),
    Text(String),
    Boolean(bool),
}

impl Scalar {
    fn arithmetic(&self) -> Option<f64> {
        match self {
            Self::Empty => Some(0.0),
            Self::Number(value) => Some(*value),
            Self::Boolean(value) => Some(f64::from(u8::from(*value))),
            Self::Text(_) => None,
        }
    }
}

#[derive(Clone)]
pub(super) enum Input {
    Value(Scalar),
    Formula(String),
    Unsupported,
}

#[derive(Clone)]
struct ResultValue {
    value: Scalar,
    affected: bool,
}

impl ResultValue {
    fn literal(value: Scalar) -> Self {
        Self {
            value,
            affected: false,
        }
    }

    fn number(value: f64, affected: bool) -> Option<Self> {
        value.is_finite().then_some(Self {
            value: Scalar::Number(value),
            affected,
        })
    }
}

type Key = (String, Coordinates);
pub(super) type Sheet = BTreeMap<Coordinates, Input>;

pub(super) struct Workbook {
    sheets: BTreeMap<String, Sheet>,
    changed: BTreeSet<Key>,
    cache: BTreeMap<Key, Option<ResultValue>>,
    resolving: BTreeSet<Key>,
    remaining: usize,
}

impl Workbook {
    pub(super) fn new(sheets: BTreeMap<String, Sheet>, changed: &BTreeSet<Key>) -> Self {
        Self {
            sheets: sheets
                .into_iter()
                .map(|(name, sheet)| (name.to_lowercase(), sheet))
                .collect(),
            changed: changed
                .iter()
                .map(|(name, cell)| (name.to_lowercase(), *cell))
                .collect(),
            cache: BTreeMap::new(),
            resolving: BTreeSet::new(),
            remaining: 1_000_000,
        }
    }

    pub(super) fn number(&mut self, sheet: &str, address: Coordinates) -> Option<(f64, bool)> {
        let result = self.cell(&sheet.to_lowercase(), address)?;
        match result.value {
            Scalar::Number(value) => Some((value, result.affected)),
            _ => None,
        }
    }

    fn cell(&mut self, sheet: &str, address: Coordinates) -> Option<ResultValue> {
        self.remaining = self.remaining.checked_sub(1)?;
        let input = self.sheets.get(sheet)?.get(&address);
        let key = (sheet.to_owned(), address);
        let changed = self.changed.contains(&key);
        match input {
            None => Some(ResultValue {
                value: Scalar::Empty,
                affected: changed,
            }),
            Some(Input::Value(value)) => Some(ResultValue {
                value: value.clone(),
                affected: changed,
            }),
            Some(Input::Unsupported) => None,
            Some(Input::Formula(formula)) => {
                if let Some(result) = self.cache.get(&key) {
                    return result.clone();
                }
                if self.resolving.len() >= 64 || !self.resolving.insert(key.clone()) {
                    return None;
                }
                let formula = formula.clone();
                let result = self.expression(&formula, sheet, 0).map(|mut result| {
                    result.affected |= changed;
                    result
                });
                self.resolving.remove(&key);
                self.cache.insert(key, result.clone());
                result
            }
        }
    }

    fn expression(&mut self, input: &str, current: &str, depth: usize) -> Option<ResultValue> {
        if depth >= 32 || input.len() > 8192 {
            return None;
        }
        let input = input.trim();
        if let Ok(number) = input.parse::<f64>() {
            return ResultValue::number(number, false);
        }
        if let Some(text) = quoted_text(input) {
            return Some(ResultValue::literal(Scalar::Text(text)));
        }
        for operators in [b"+-", b"*/"] {
            let terms = split(input, operators)?;
            if terms.len() < 2 {
                continue;
            }
            let mut terms = terms.into_iter();
            let mut result = self.expression(terms.next()?.1, current, depth + 1)?;
            for (operator, input) in terms {
                let right = self.expression(input, current, depth + 1)?;
                let lhs = result.value.arithmetic()?;
                let rhs = right.value.arithmetic()?;
                let number = match operator {
                    b'+' => lhs + rhs,
                    b'-' => lhs - rhs,
                    b'*' => lhs * rhs,
                    b'/' => lhs / rhs,
                    _ => unreachable!("formula operator"),
                };
                result = ResultValue::number(number, result.affected || right.affected)?;
            }
            return Some(result);
        }
        if let Some(input) = input.strip_prefix('+') {
            let value = self.expression(input, current, depth + 1)?;
            return ResultValue::number(value.value.arithmetic()?, value.affected);
        }
        if let Some(input) = input.strip_prefix('-') {
            let value = self.expression(input, current, depth + 1)?;
            return ResultValue::number(-value.value.arithmetic()?, value.affected);
        }
        if let Some(inner) = input
            .strip_prefix('(')
            .and_then(|text| text.strip_suffix(')'))
        {
            return self.expression(inner, current, depth + 1);
        }
        if let Some((sheet, (start, end))) = sheet_range(input, current) {
            if start == end {
                return self.cell(&sheet.to_lowercase(), start);
            }
        }
        let (name, arguments) = input.strip_suffix(')')?.split_once('(')?;
        let arguments: Vec<_> = split(arguments, b",")?
            .into_iter()
            .map(|(_, text)| text.trim())
            .collect();
        match (
            name.trim().to_ascii_uppercase().as_str(),
            arguments.as_slice(),
        ) {
            (name @ ("SUM" | "COUNT" | "AVERAGE"), [range]) => self.aggregate(name, range, current),
            ("SUMIFS", [sum, conditions @ ..]) => {
                self.conditional(Some(sum), conditions, current, depth + 1)
            }
            ("COUNTIFS", conditions) => self.conditional(None, conditions, current, depth + 1),
            _ => None,
        }
    }

    fn aggregate(&mut self, name: &str, range: &str, current: &str) -> Option<ResultValue> {
        let (sheet, bounds) = sheet_range(range, current)?;
        let sheet = sheet.to_lowercase();
        let addresses: Vec<_> = self
            .sheets
            .get(&sheet)?
            .range(bounds.0..=bounds.1)
            .map(|(&cell, _)| cell)
            .filter(|&cell| within(cell, bounds))
            .collect();
        let mut sum = 0.0;
        let mut count = 0;
        let mut affected = false;
        for cell in addresses {
            let result = self.cell(&sheet, cell)?;
            affected |= result.affected;
            if let Scalar::Number(value) = result.value {
                sum += value;
                count += 1;
            }
        }
        let number = match name {
            "SUM" => sum,
            "COUNT" => f64::from(count),
            _ => sum / f64::from(count),
        };
        ResultValue::number(number, affected)
    }

    fn conditional(
        &mut self,
        sum: Option<&str>,
        args: &[&str],
        current: &str,
        depth: usize,
    ) -> Option<ResultValue> {
        if args.is_empty() || args.len() > 16 || args.len() % 2 != 0 {
            return None;
        }
        let mut conditions = Vec::new();
        let mut affected = false;
        for pair in args.chunks_exact(2) {
            let range = self.range(pair[0], current)?;
            let criterion = self.expression(pair[1], current, depth)?;
            if !exact_criterion(&criterion.value) {
                return None;
            }
            affected |= criterion.affected;
            conditions.push((range, criterion.value));
        }
        let total_range = match sum {
            Some(range) => Some(self.range(range, current)?),
            None => None,
        };
        let (rows, columns) = conditions[0].0.shape();
        if u64::from(rows) * u64::from(columns) > 20_000
            || conditions
                .iter()
                .any(|(range, _)| range.shape() != (rows, columns))
            || total_range
                .as_ref()
                .is_some_and(|range| range.shape() != (rows, columns))
        {
            return None;
        }
        let mut total = 0.0;
        for row in 0..rows {
            for column in 0..columns {
                let mut matches = true;
                for (range, expected) in &conditions {
                    let actual = self.cell(&range.sheet, range.at(row, column))?;
                    affected |= actual.affected;
                    matches &= equal_criterion(&actual.value, expected)?;
                }
                if matches {
                    if let Some(range) = &total_range {
                        let value = self.cell(&range.sheet, range.at(row, column))?;
                        affected |= value.affected;
                        if let Scalar::Number(value) = value.value {
                            total += value;
                        }
                    } else {
                        total += 1.0;
                    }
                }
            }
        }
        ResultValue::number(total, affected)
    }

    fn range(&self, reference: &str, current: &str) -> Option<Reference> {
        let (sheet, bounds) = sheet_range(reference, current)?;
        let sheet = sheet.to_lowercase();
        self.sheets
            .contains_key(&sheet)
            .then_some(Reference { sheet, bounds })
    }
}

struct Reference {
    sheet: String,
    bounds: CellRange,
}

impl Reference {
    fn shape(&self) -> (u32, u32) {
        let ((top, left), (bottom, right)) = self.bounds;
        (bottom - top + 1, right - left + 1)
    }

    fn at(&self, row: u32, column: u32) -> Coordinates {
        let (top, left) = self.bounds.0;
        (top + row, left + column)
    }
}

fn exact_criterion(value: &Scalar) -> bool {
    match value {
        Scalar::Text(text) => {
            !text.is_empty()
                && !text.starts_with(['=', '<', '>'])
                && !text.contains(['?', '*', '~'])
                && text.parse::<f64>().is_err()
        }
        Scalar::Number(_) | Scalar::Boolean(_) => true,
        Scalar::Empty => false,
    }
}

fn equal_criterion(value: &Scalar, expected: &Scalar) -> Option<bool> {
    match (value, expected) {
        (Scalar::Text(value), Scalar::Text(expected)) => {
            Some(value.to_lowercase() == expected.to_lowercase())
        }
        (Scalar::Number(value), Scalar::Number(expected)) => Some(value == expected),
        (Scalar::Boolean(value), Scalar::Boolean(expected)) => Some(value == expected),
        (Scalar::Text(value), Scalar::Number(_)) if value.parse::<f64>().is_ok() => None,
        (Scalar::Empty, Scalar::Number(value)) if *value == 0.0 => None,
        _ => Some(false),
    }
}

fn quoted_text(input: &str) -> Option<String> {
    let text = input.strip_prefix('"')?.strip_suffix('"')?;
    let mut chars = text.chars();
    let mut value = String::new();
    while let Some(character) = chars.next() {
        if character == '"' && chars.next() != Some('"') {
            return None;
        }
        value.push(character);
    }
    Some(value)
}

// Separators inside quoted sheet names, criteria, and function calls stay
// with their argument. Consecutive signs remain with the unary operand.
fn split<'a>(input: &'a str, separators: &[u8]) -> Option<Vec<(u8, &'a str)>> {
    let mut result = Vec::new();
    let mut depth = 0_u32;
    let mut quote = None;
    let mut start = 0;
    let mut operator = 0;
    let mut last = None;
    for (index, byte) in input.bytes().enumerate() {
        if let Some(delimiter) = quote {
            if byte == delimiter {
                quote = None;
                last = Some(byte);
            }
            continue;
        }
        match byte {
            b'\'' | b'"' => quote = Some(byte),
            b'(' => depth += 1,
            b')' => depth = depth.checked_sub(1)?,
            byte if depth == 0 && separators.contains(&byte) => {
                let unary = matches!(byte, b'+' | b'-')
                    && matches!(last, None | Some(b'+' | b'-' | b'*' | b'/' | b',' | b'('));
                if !unary {
                    result.push((operator, &input[start..index]));
                    operator = byte;
                    start = index + 1;
                }
            }
            _ => {}
        }
        if !byte.is_ascii_whitespace() {
            last = Some(byte);
        }
    }
    if quote.is_some() || depth != 0 {
        return None;
    }
    result.push((operator, &input[start..]));
    Some(result)
}
