//! Decision-assistance evaluation — the offline half.
//!
//! This layer measures the **local rule**, not the external model. It
//! answers the question the quality gate needs before any cloud call is
//! allowed: on a frozen fictional corpus, how often is the local rule
//! right, how often is it unsure, and what would the gate's thresholds
//! look like if a model answered every ambiguous case?
//!
//! Nothing here talks to the network. The model comparison is `NOT RUN`:
//! it needs a separately authorized corpus run against the real service.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use kodo_agent::classify_task_with_evidence;
use kodo_agent::classify_verify_failure;
use kodo_agent::decision::{
    build_request, plan_failure_triage, plan_task_route, FailureTriageInput, TaskRouteInput,
};
use kodo_agent::triage_verify_failure;
use kodo_agent::{TurnFailureClass, TurnFailureTriage as FailureTriage, ROUTE_LABELS};

/// One corpus line.
#[derive(Debug, Clone, serde::Deserialize)]
struct Sample {
    id: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    command_category: String,
    #[serde(default)]
    exit_code: Option<i32>,
    #[serde(default)]
    known_flags: Vec<String>,
    #[serde(default)]
    error_excerpt: String,
    label: String,
    #[serde(default)]
    split: String,
    #[serde(default)]
    note: String,
}

/// The measured result of one corpus.
#[derive(Debug, Default, Clone)]
pub struct CorpusResult {
    /// Samples that carry a label inside the vocabulary — the denominator
    /// every rate below uses.
    pub total: usize,
    pub correct: usize,
    /// The label set the macro F1 is averaged over.
    pub labels: Vec<String>,
    /// Per-label true positives / false positives / false negatives.
    pub per_label: BTreeMap<String, (usize, usize, usize)>,
    /// Per-label `(truth count, count the rule got right)`: the denominator
    /// a per-category number needs, because the total hides a class the rule
    /// never reaches.
    pub per_label_support: BTreeMap<String, (usize, usize)>,
    /// Per split (`tune` / `holdout`) `(total, correct)`.
    pub per_split: BTreeMap<String, (usize, usize)>,
    /// How many samples the local rule was unsure about.
    pub ambiguous: usize,
    /// How many of the unsure samples the local rule still got right.
    pub ambiguous_correct: usize,
    /// How many samples the local rule marked as an environment problem
    /// while the label says the product is at fault. This is the rate that
    /// would *block* work that could have been finished.
    pub product_blocked: usize,
    /// Samples whose label is not in the vocabulary. They are reported, not
    /// silently counted as rule misses: a label the rule cannot name is an
    /// evaluation defect, not a measurement.
    pub off_vocabulary: Vec<String>,
    pub notes: Vec<String>,
}

impl CorpusResult {
    pub fn accuracy(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        self.correct as f64 / self.total as f64
    }

    /// The macro F1 over the label set. A label nobody predicted still
    /// counts: a class that is always missed must show up as a zero.
    pub fn macro_f1(&self) -> f64 {
        if self.labels.is_empty() {
            return 0.0;
        }
        let mut total = 0.0;
        for label in &self.labels {
            let (tp, fp, fn_) = self.per_label.get(label).copied().unwrap_or((0, 0, 0));
            let precision = if tp + fp == 0 {
                0.0
            } else {
                tp as f64 / (tp + fp) as f64
            };
            let recall = if tp + fn_ == 0 {
                0.0
            } else {
                tp as f64 / (tp + fn_) as f64
            };
            let f1 = if precision + recall == 0.0 {
                0.0
            } else {
                2.0 * precision * recall / (precision + recall)
            };
            total += f1;
        }
        total / self.labels.len() as f64
    }

    /// The share of the corpus the local rule was unsure about.
    pub fn ambiguous_rate(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        self.ambiguous as f64 / self.total as f64
    }
}

/// The task-route label vocabulary. Not a local copy: this is the same list
/// the runtime's own `route_label` can produce and the decision assistant
/// offers, so a class the rule cannot name is impossible here.
fn route_labels() -> Vec<String> {
    ROUTE_LABELS
        .iter()
        .map(|label| (*label).to_owned())
        .collect()
}

/// The failure-triage label vocabulary, taken from the enum that produces the
/// predictions rather than restated here.
fn failure_labels() -> Vec<String> {
    FailureTriage::ALL
        .iter()
        .map(|triage| triage.label().to_owned())
        .collect()
}

/// The corpus shape: what the file holds and whether all of it could be read.
#[derive(Debug, Default, Clone)]
struct Corpus {
    samples: Vec<Sample>,
    /// Non-empty lines in the file.
    raw_lines: usize,
    /// Lines that are not a sample object at all.
    malformed: usize,
    /// Labels outside the vocabulary.
    unknown_labels: Vec<String>,
    duplicate_ids: Vec<String>,
}

impl Corpus {
    fn loaded(&self) -> usize {
        self.samples.len()
    }
}

fn corpus_dir() -> Option<PathBuf> {
    // The corpus lives next to the crate, in its own directory: the
    // fixture layer scans `evals/fixtures/` and expects every entry there
    // to be a task repo with an oracle.
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = manifest.join("fixtures-decision");
    dir.is_dir().then_some(dir)
}

fn load(name: &str, labels: &[String]) -> Corpus {
    let Some(dir) = corpus_dir() else {
        return Corpus::default();
    };
    let Ok(text) = std::fs::read_to_string(dir.join(name)) else {
        return Corpus::default();
    };
    parse_corpus(&text, labels)
}

/// Read a corpus, counting what could not be read.
///
/// A dropped line used to be invisible: the metrics were then computed over
/// an unknown denominator. Every line is accounted for here, and a label
/// outside the vocabulary is named instead of scored as a rule miss.
fn parse_corpus(text: &str, labels: &[String]) -> Corpus {
    let mut corpus = Corpus::default();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        corpus.raw_lines += 1;
        let Ok(sample) = serde_json::from_str::<Sample>(line) else {
            corpus.malformed += 1;
            continue;
        };
        *seen.entry(sample.id.clone()).or_default() += 1;
        if !labels.iter().any(|label| *label == sample.label) {
            corpus
                .unknown_labels
                .push(format!("{}:{}", sample.id, sample.label));
        }
        corpus.samples.push(sample);
    }
    corpus.duplicate_ids = seen
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(id, count)| format!("{id}×{count}"))
        .collect();
    corpus
}

/// Score one corpus with one prediction rule.
///
/// `unsure` decides what counts as the local rule declining to attribute
/// anything — that is corpus-specific, so it is a parameter instead of a
/// hard-coded label check. A label outside `labels` is excluded from the
/// denominator and reported: measuring a class the rule cannot name would
/// blame the rule for the evaluation's own gap.
fn score_corpus(
    samples: &[Sample],
    labels: &[String],
    predict: &dyn Fn(&Sample) -> String,
    unsure: &dyn Fn(&Sample, &str) -> bool,
) -> CorpusResult {
    let mut result = CorpusResult {
        labels: labels.to_vec(),
        ..CorpusResult::default()
    };
    for sample in samples {
        let predicted = predict(sample);
        let truth = sample.label.as_str();
        if !labels.iter().any(|label| label == truth) {
            result
                .off_vocabulary
                .push(format!("{}: {truth}", sample.id));
            continue;
        }
        result.total += 1;
        if unsure(sample, &predicted) {
            result.ambiguous += 1;
            if predicted == truth {
                result.ambiguous_correct += 1;
            }
        }
        if predicted == truth {
            result.correct += 1;
            result
                .per_label
                .entry(truth.to_owned())
                .or_insert((0, 0, 0))
                .0 += 1;
        } else {
            result
                .per_label
                .entry(predicted.clone())
                .or_insert((0, 0, 0))
                .1 += 1;
            result
                .per_label
                .entry(truth.to_owned())
                .or_insert((0, 0, 0))
                .2 += 1;
        }
        let support = result
            .per_label_support
            .entry(truth.to_owned())
            .or_insert((0, 0));
        support.0 += 1;
        if predicted == truth {
            support.1 += 1;
        }
        let split = result
            .per_split
            .entry(if sample.split.is_empty() {
                "(no split)".to_owned()
            } else {
                sample.split.clone()
            })
            .or_insert((0, 0));
        split.0 += 1;
        if predicted == truth {
            split.1 += 1;
        }
        // The dangerous direction: the rule says the environment is at fault
        // while the label says the product is.
        if predicted == "infrastructure" && truth == "product" {
            result.product_blocked += 1;
        }
    }
    if result.total == 0 {
        result
            .notes
            .push("no sample in this corpus could be scored".to_owned());
    }
    result
}

/// The local rule's route, in the vocabulary the decision assistant uses: a
/// request no keyword reaches is `unknown`, not a guess at `feature`.
fn measure_task_route(samples: &[Sample]) -> CorpusResult {
    score_corpus(
        samples,
        &route_labels(),
        &|sample| {
            classify_task_with_evidence(&sample.message)
                .route_label()
                .to_owned()
        },
        &|sample, _| classify_task_with_evidence(&sample.message).ambiguous,
    )
}

/// The previous 口径, kept so the number before this change stays comparable:
/// the rule always named a category, so `unknown` was unreachable by
/// construction and every vague request was counted as a `feature` guess.
fn measure_task_route_legacy(samples: &[Sample]) -> CorpusResult {
    score_corpus(
        samples,
        &route_labels(),
        &|sample| {
            classify_task_with_evidence(&sample.message)
                .baseline
                .label()
                .to_owned()
        },
        &|sample, _| classify_task_with_evidence(&sample.message).ambiguous,
    )
}

/// The command line a corpus category stands for.
///
/// The corpus records a category, not a command; the runtime always has the
/// command line, so the scorer reconstructs the shape it would have seen.
/// `generic` stays empty on purpose — inventing a command there would be a
/// guess the runtime never made.
fn command_line(category: &str) -> &'static str {
    match category {
        "rust_check" => "cargo check -p kodo-agent",
        "rust_test" => "cargo test -p kodo-agent",
        "frontend_typecheck" => "npm run typecheck",
        "frontend_test" => "npm run test",
        _ => "",
    }
}

fn has_flag(flags: &[String], name: &str) -> bool {
    flags.iter().any(|flag| flag == name)
}

/// The evidence-tiered triage: process state and exit code first, then the
/// output's own diagnostic shapes.
fn measure_failure_triage(samples: &[Sample]) -> CorpusResult {
    score_corpus(
        samples,
        &failure_labels(),
        &|sample| {
            triage_verify_failure(
                command_line(&sample.command_category),
                sample.exit_code,
                has_flag(&sample.known_flags, "timed_out"),
                has_flag(&sample.known_flags, "cancelled"),
                &sample.error_excerpt,
            )
            .label()
            .to_owned()
        },
        &|_, predicted| predicted == "unknown",
    )
}

/// The previous 口径: the persisted marker, with `Timeout` renamed to
/// `transient` and everything else — including a bare non-zero exit — called
/// `product`. It has no `unknown` bucket at all.
fn measure_failure_triage_legacy(samples: &[Sample]) -> CorpusResult {
    score_corpus(
        samples,
        &failure_labels(),
        &|sample| match classify_verify_failure(&sample.command_category, &sample.error_excerpt) {
            TurnFailureClass::Infrastructure => "infrastructure".to_owned(),
            TurnFailureClass::Timeout => "transient".to_owned(),
            _ => "product".to_owned(),
        },
        &|sample, predicted| predicted == "product" && sample.label != "product",
    )
}

/// How many samples the local rule would call unsure at a given margin.
///
/// The margin constant is a *policy* value, not a measurement: the design
/// proposes 4 and explicitly leaves it to be evaluated. This sweep is the
/// offline half of that evaluation — it says how much of the corpus the
/// decision component would be asked about at each setting.
fn ambiguous_by_margin(samples: &[Sample], margin: u32) -> usize {
    samples
        .iter()
        .filter(|sample| {
            let evidence = classify_task_with_evidence(&sample.message);
            if !evidence.ambiguous {
                return false;
            }
            // Re-derive the ambiguity with the requested margin.
            let top = evidence.top_score;
            let runner_up = evidence.runner_up_score;
            let tied = evidence
                .scores
                .iter()
                .skip(1)
                .find(|(_, score)| *score > 0)
                .is_some_and(|(_, score)| *score == top);
            let default_fallback = evidence.hit_count == 0;
            default_fallback || tied || top.saturating_sub(runner_up) < margin
        })
        .count()
}

/// The offline contract check: the state that would be sent must contain
/// exactly the fields the design allows, and nothing else.
///
/// This is the check that can be made without a network call, and it is the
/// one that matters most for the data-minimisation promise: a field that
/// sneaks into the state is a field that can leave the machine.
fn check_state_contract(samples: &[Sample]) -> (usize, usize) {
    let mut ok = 0usize;
    let mut total = 0usize;
    for sample in samples {
        let plan = plan_task_route(&TaskRouteInput {
            message: sample.message.clone(),
            mode: "code".to_owned(),
            baseline: kodo_agent::TaskType::Feature,
        });
        let request = build_request("jev-1.13.0", &plan);
        let state = request.state.as_object().cloned().unwrap_or_default();
        let mut keys: Vec<&String> = state.keys().collect();
        keys.sort();
        total += 1;
        if keys == vec!["message", "mode"] {
            ok += 1;
        }
    }
    for sample in samples {
        let plan = plan_failure_triage(&FailureTriageInput {
            command_category: sample.command_category.clone(),
            exit_code: sample.exit_code,
            known_flags: sample.known_flags.clone(),
            error_excerpt: sample.error_excerpt.clone(),
        });
        let request = build_request("jev-1.13.0", &plan);
        let state = request.state.as_object().cloned().unwrap_or_default();
        let mut keys: Vec<&String> = state.keys().collect();
        keys.sort();
        total += 1;
        if keys == vec!["commandCategory", "errorExcerpt", "exitCode", "knownFlags"] {
            ok += 1;
        }
    }
    (ok, total)
}

/// The samples the local rule got wrong, with their corpus ids. The ids
/// are what a corpus curator needs to find the sample again.
fn print_misses(
    samples: &[Sample],
    title: &str,
    limit: usize,
    predict: &dyn Fn(&Sample) -> String,
) {
    let mut shown = 0usize;
    for sample in samples {
        if shown >= limit {
            break;
        }
        let predicted = predict(sample);
        if predicted == sample.label {
            continue;
        }
        shown += 1;
        let text: String = if sample.message.is_empty() {
            sample.error_excerpt.chars().take(48).collect()
        } else {
            sample.message.chars().take(48).collect()
        };
        println!(
            "  {title} miss {} [{}]: predicted={predicted} truth={} \"{text}\"{}",
            sample.id,
            sample.split,
            sample.label,
            if sample.note.is_empty() {
                String::new()
            } else {
                format!(" ({})", sample.note)
            }
        );
    }
}

/// One corpus, reported by category and by split — not as one total.
fn print_metrics(prefix: &str, result: &CorpusResult) {
    println!(
        "{prefix}_accuracy: {:.3} ({}/{})",
        result.accuracy(),
        result.correct,
        result.total
    );
    println!("{prefix}_macro_f1: {:.3}", result.macro_f1());
    for (split, (total, correct)) in &result.per_split {
        let rate = if *total == 0 {
            0.0
        } else {
            *correct as f64 / *total as f64
        };
        println!("  {prefix}[split {split}]: accuracy={rate:.3} ({correct}/{total})");
    }
    for label in &result.labels {
        let (tp, fp, fn_) = result.per_label.get(label).copied().unwrap_or((0, 0, 0));
        let (support, right) = result
            .per_label_support
            .get(label)
            .copied()
            .unwrap_or((0, 0));
        println!(
            "  {prefix}[{label}]: tp={tp} fp={fp} fn={fn_} — category accuracy={right}/{support}"
        );
    }
    println!(
        "  {prefix}_ambiguous: {} ({:.1}% of the corpus, {} of them right)",
        result.ambiguous,
        result.ambiguous_rate() * 100.0,
        result.ambiguous_correct
    );
    println!(
        "  {prefix}_product_blocked: {} ({}/{}) — a product failure the rule would call an environment failure",
        result.product_blocked,
        result.product_blocked,
        result.total
    );
    if !result.off_vocabulary.is_empty() {
        println!(
            "  {prefix}_off_vocabulary: {} sample(s) carry a label the rule cannot name: {}",
            result.off_vocabulary.len(),
            result.off_vocabulary.join(", ")
        );
    }
    for note in &result.notes {
        println!("  {prefix}_note: {note}");
    }
}

/// Run the offline decision evaluation and print its report.
/// Report what was actually read. A corpus that silently loses a line is a
/// corpus whose metrics mean nothing.
fn print_corpus(name: &str, corpus: &Corpus) {
    println!(
        "  {name}: {} line(s) read, {} sample(s) parsed, {} malformed, {} duplicated id(s)",
        corpus.raw_lines,
        corpus.loaded(),
        corpus.malformed,
        corpus.duplicate_ids.len()
    );
    if !corpus.duplicate_ids.is_empty() {
        println!(
            "    {name} duplicate ids: {}",
            corpus.duplicate_ids.join(", ")
        );
    }
    if !corpus.unknown_labels.is_empty() {
        println!(
            "    {name} labels outside the vocabulary: {}",
            corpus.unknown_labels.join(", ")
        );
    }
}

pub fn run() {
    println!("\nkodo-evals decision layer (offline, local rules only)");
    let task = load("task_route.jsonl", &route_labels());
    let failure = load("failure_triage.jsonl", &failure_labels());
    let task_samples = &task.samples;
    let failure_samples = &failure.samples;
    let predict_task = |sample: &Sample| {
        classify_task_with_evidence(&sample.message)
            .route_label()
            .to_owned()
    };
    let predict_failure = |sample: &Sample| {
        triage_verify_failure(
            command_line(&sample.command_category),
            sample.exit_code,
            has_flag(&sample.known_flags, "timed_out"),
            has_flag(&sample.known_flags, "cancelled"),
            &sample.error_excerpt,
        )
        .label()
        .to_owned()
    };

    println!("\n--- decision · corpus integrity (frozen: nothing here is rewritten) ---");
    print_corpus("task_route", &task);
    print_corpus("failure_triage", &failure);

    println!("\n--- decision · task route (local rule baseline) ---");
    let task_splits: BTreeMap<String, usize> =
        task_samples
            .iter()
            .fold(BTreeMap::new(), |mut map, sample| {
                *map.entry(sample.split.clone()).or_default() += 1;
                map
            });
    let task_splits = task_splits
        .iter()
        .map(|(split, count)| format!("{split}={count}"))
        .collect::<Vec<_>>()
        .join(" ");
    println!("task_route_samples: {} ({task_splits})", task_samples.len());
    let task = measure_task_route(task_samples);
    print_metrics("task_route", &task);
    let task_legacy = measure_task_route_legacy(task_samples);
    println!("task_route_legacy (previous 口径, `unknown` unreachable by construction):");
    print_metrics("task_route_legacy", &task_legacy);
    println!("task_route_margin_sweep (ambiguous samples per margin):");
    for margin in [0u32, 2, 4, 8, 12] {
        let count = ambiguous_by_margin(task_samples, margin);
        println!(
            "  margin={margin}: {count} ({:.1}%)",
            if task.total == 0 {
                0.0
            } else {
                count as f64 / task.total as f64 * 100.0
            }
        );
    }

    println!("\n--- decision · failure triage (local rule baseline) ---");
    let failure_splits: BTreeMap<String, usize> =
        failure_samples
            .iter()
            .fold(BTreeMap::new(), |mut map, sample| {
                *map.entry(sample.split.clone()).or_default() += 1;
                map
            });
    let failure_splits = failure_splits
        .iter()
        .map(|(split, count)| format!("{split}={count}"))
        .collect::<Vec<_>>()
        .join(" ");
    println!(
        "failure_triage_samples: {} ({failure_splits})",
        failure_samples.len()
    );
    let failure = measure_failure_triage(failure_samples);
    print_metrics("failure_triage", &failure);
    let failure_legacy = measure_failure_triage_legacy(failure_samples);
    println!("failure_triage_legacy (previous 口径, no `unknown` bucket at all):");
    print_metrics("failure_triage_legacy", &failure_legacy);

    println!("task_route_misses (first 10):");
    print_misses(task_samples, "task", 10, &predict_task);
    println!("failure_triage_misses (first 10):");
    print_misses(failure_samples, "failure", 10, &predict_failure);

    println!("\n--- decision · offline contract check ---");
    let (mut contract_ok, mut contract_total) = check_state_contract(task_samples);
    let (failure_ok, failure_total) = check_state_contract(failure_samples);
    contract_ok += failure_ok;
    contract_total += failure_total;
    println!(
        "state_contract_ok: {contract_ok}/{contract_total} (the state carries only the allowed fields)"
    );

    println!("\n--- decision · model comparison ---");
    println!("model_comparison: NOT RUN — a cloud evaluation needs a separate authorization");
    println!("quality_gate: NOT EVALUATED — the offline numbers above are the local rule's, not the model's");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(message: &str, label: &str) -> Sample {
        Sample {
            id: "t".to_owned(),
            message: message.to_owned(),
            command_category: String::new(),
            exit_code: None,
            known_flags: Vec::new(),
            error_excerpt: String::new(),
            label: label.to_owned(),
            split: "tune".to_owned(),
            note: String::new(),
        }
    }

    #[test]
    fn a_perfect_rule_scores_one() {
        let samples = vec![sample("修复崩溃", "bug-fix"), sample("新增导出", "feature")];
        let result = measure_task_route(&samples);
        assert_eq!(result.total, 2);
        assert_eq!(result.correct, 2);
        assert_eq!(result.accuracy(), 1.0);
    }

    #[test]
    fn a_request_the_rules_cannot_place_is_named_unknown() {
        // Before, the scorer used `baseline.label()`, which can only name the
        // six keyword categories, so every `unknown` sample was a structural
        // miss no matter what the message said.
        let samples = vec![
            sample("新增导出", "feature"),
            sample("把那个东西弄好", "unknown"),
        ];
        let result = measure_task_route(&samples);
        assert_eq!(result.correct, 2, "{:?}", result.per_label);
        assert_eq!(result.per_label.get("unknown").copied().unwrap().0, 1);
        // And it is still reported as an unsure case, not as a confident hit.
        assert_eq!(result.ambiguous, 2);
    }

    #[test]
    fn the_legacy_number_is_still_measurable() {
        // The comparability record: the old 口径 could never reach `unknown`.
        let samples = vec![sample("把那个东西弄好", "unknown")];
        assert_eq!(measure_task_route_legacy(&samples).correct, 0);
        assert_eq!(measure_task_route(&samples).correct, 1);
    }

    #[test]
    fn the_route_vocabulary_covers_every_label_the_rule_can_name() {
        let labels = route_labels();
        for message in [
            "修复崩溃",
            "新增导出",
            "给 plan 模块补测试用例",
            "重构上下文扫描逻辑",
            "审查一下这个 PR",
            "补充 API 使用文档",
            "把那个东西弄好",
            "",
            "保存到 docs/架构.md",
        ] {
            let predicted = classify_task_with_evidence(message).route_label();
            assert!(labels.iter().any(|label| label == predicted), "{message}");
        }
    }

    #[test]
    fn a_label_outside_the_vocabulary_is_reported_not_scored() {
        // `work` is a mode tag, never a classification result — a corpus that
        // carried it must not be blamed on the rule.
        let samples = vec![
            sample("修复崩溃", "bug-fix"),
            sample("总结这次改动", "work"),
        ];
        let result = measure_task_route(&samples);
        assert_eq!(result.total, 1, "only the scoreable samples count");
        assert_eq!(result.off_vocabulary.len(), 1);
        assert!(result.off_vocabulary[0].contains("work"));
    }

    #[test]
    fn metrics_are_kept_per_split_and_per_category() {
        let mut vague = sample("把那个东西弄好", "unknown");
        vague.split = "holdout".to_owned();
        let result = measure_task_route(&[sample("修复崩溃", "bug-fix"), vague]);
        assert_eq!(result.per_split.get("tune"), Some(&(1, 1)));
        assert_eq!(result.per_split.get("holdout"), Some(&(1, 1)));
        assert_eq!(result.per_label_support.get("bug-fix"), Some(&(1, 1)));
        assert_eq!(result.per_label_support.get("unknown"), Some(&(1, 1)));
    }

    #[test]
    fn a_dropped_corpus_line_is_counted() {
        let text = r#"{"id":"t-1","message":"修复崩溃","label":"bug-fix","split":"tune"}
{"id":"t-1","message":"新增导出","label":"feature","split":"tune"}
this line is not a sample at all
{"id":"t-2","message":"总结","label":"work","split":"holdout"}"#;
        let corpus = parse_corpus(text, &route_labels());
        assert_eq!(corpus.raw_lines, 4);
        assert_eq!(corpus.loaded(), 3);
        assert_eq!(corpus.malformed, 1);
        assert_eq!(corpus.duplicate_ids, vec!["t-1×2".to_owned()]);
        assert_eq!(corpus.unknown_labels, vec!["t-2:work".to_owned()]);
    }

    #[test]
    fn a_generic_sample_does_not_invent_a_command_line() {
        assert_eq!(command_line("generic"), "");
        assert!(command_line("rust_test").starts_with("cargo test"));
        assert!(command_line("frontend_typecheck").starts_with("npm run"));
    }

    #[test]
    fn an_environment_failure_is_not_a_product_block() {
        let samples = vec![Sample {
            id: "f".to_owned(),
            message: String::new(),
            command_category: "generic".to_owned(),
            exit_code: Some(1),
            known_flags: vec![],
            error_excerpt: "command not found".to_owned(),
            label: "product".to_owned(),
            split: "tune".to_owned(),
            note: String::new(),
        }];
        let result = measure_failure_triage(&samples);
        assert_eq!(result.product_blocked, 1);
    }

    #[test]
    fn a_transient_process_state_is_not_called_product() {
        let failure = |flags: Vec<String>, label: &str| Sample {
            id: "f".to_owned(),
            message: String::new(),
            command_category: "rust_test".to_owned(),
            exit_code: Some(101),
            known_flags: flags,
            error_excerpt: String::new(),
            label: label.to_owned(),
            split: "tune".to_owned(),
            note: String::new(),
        };
        for flags in [vec!["timed_out".to_owned()], vec!["cancelled".to_owned()]] {
            let result = measure_failure_triage(&[failure(flags.clone(), "transient")]);
            assert_eq!(result.correct, 1, "{flags:?} must be transient");
        }
    }

    #[test]
    fn the_frozen_triage_corpus_reaches_every_class_it_claims() {
        let corpus = load("failure_triage.jsonl", &failure_labels());
        let result = measure_failure_triage(&corpus.samples);
        assert_eq!(corpus.malformed, 0, "the frozen corpus must still parse");
        assert_eq!(result.total, corpus.loaded(), "every line is scored");
        assert_eq!(result.off_vocabulary, Vec::<String>::new());
        // The direction that strands work: never call a product failure an
        // environment failure.
        assert_eq!(result.product_blocked, 0);
        for label in failure_labels() {
            let (tp, _, _) = result.per_label.get(&label).copied().unwrap_or((0, 0, 0));
            assert!(tp > 0, "{label} is unreachable for the rule");
        }
    }

    #[test]
    fn the_frozen_route_corpus_can_reach_unknown() {
        let corpus = load("task_route.jsonl", &route_labels());
        let result = measure_task_route(&corpus.samples);
        assert_eq!(result.total, corpus.loaded());
        assert_eq!(result.off_vocabulary, Vec::<String>::new());
        let (tp, _, _) = result
            .per_label
            .get("unknown")
            .copied()
            .unwrap_or((0, 0, 0));
        assert!(tp > 0, "the 20 `unknown` samples must be reachable");
        // The old 口径 could not reach this class at all.
        let legacy = measure_task_route_legacy(&corpus.samples);
        assert_eq!(
            legacy
                .per_label
                .get("unknown")
                .copied()
                .unwrap_or((0, 0, 0))
                .0,
            0
        );
        assert!(result.accuracy() >= legacy.accuracy());
    }
}
