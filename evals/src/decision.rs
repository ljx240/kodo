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
use kodo_agent::TurnFailureClass;

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
    pub total: usize,
    pub correct: usize,
    /// The label set the macro F1 is averaged over.
    pub labels: Vec<String>,
    /// Per-label true positives / false positives / false negatives.
    pub per_label: BTreeMap<String, (usize, usize, usize)>,
    /// How many samples the local rule was unsure about.
    pub ambiguous: usize,
    /// How many of the unsure samples the local rule still got right.
    pub ambiguous_correct: usize,
    /// How many samples the local rule marked as an environment problem
    /// while the label says the product is at fault. This is the rate that
    /// would *block* work that could have been finished.
    pub product_blocked: usize,
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

/// The task-route labels the corpus uses.
const TASK_LABELS: [&str; 7] = [
    "bug-fix",
    "feature",
    "test",
    "refactor",
    "code-review",
    "docs",
    "unknown",
];

/// The failure-triage labels the corpus uses.
const FAILURE_LABELS: [&str; 4] = ["product", "infrastructure", "transient", "unknown"];

fn corpus_dir() -> Option<PathBuf> {
    // The corpus lives next to the crate, in its own directory: the
    // fixture layer scans `evals/fixtures/` and expects every entry there
    // to be a task repo with an oracle.
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = manifest.join("fixtures-decision");
    dir.is_dir().then_some(dir)
}

fn load(name: &str) -> Vec<Sample> {
    let Some(dir) = corpus_dir() else {
        return Vec::new();
    };
    let path = dir.join(name);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<Sample>(line).ok())
        .collect()
}

/// Measure the local rule on the task-route corpus.
fn measure_task_route(samples: &[Sample]) -> CorpusResult {
    let mut result = CorpusResult {
        total: samples.len(),
        labels: TASK_LABELS
            .iter()
            .map(|label| (*label).to_owned())
            .collect(),
        ..CorpusResult::default()
    };
    for sample in samples {
        let evidence = classify_task_with_evidence(&sample.message);
        let predicted = evidence.baseline.label();
        let truth = sample.label.as_str();
        if evidence.ambiguous {
            result.ambiguous += 1;
            if predicted == truth {
                result.ambiguous_correct += 1;
            }
        }
        if predicted == truth {
            result.correct += 1;
        }
        if predicted == truth {
            result
                .per_label
                .entry(truth.to_owned())
                .or_insert((0, 0, 0))
                .0 += 1;
        } else {
            result
                .per_label
                .entry(predicted.to_owned())
                .or_insert((0, 0, 0))
                .1 += 1;
            result
                .per_label
                .entry(truth.to_owned())
                .or_insert((0, 0, 0))
                .2 += 1;
        }
    }
    if result.total == 0 {
        result
            .notes
            .push("the task-route corpus is missing".to_owned());
    }
    result
}

/// Measure the local rule on the failure-triage corpus.
fn measure_failure_triage(samples: &[Sample]) -> CorpusResult {
    let mut result = CorpusResult {
        total: samples.len(),
        labels: FAILURE_LABELS
            .iter()
            .map(|label| (*label).to_owned())
            .collect(),
        ..CorpusResult::default()
    };
    for sample in samples {
        // The local rule looks at the command and the output, exactly as the
        // runtime does. The raw command line is never sent anywhere; here
        // it is only the locally derived category plus the excerpt.
        let command = sample.command_category.clone();
        let predicted = match classify_verify_failure(&command, &sample.error_excerpt) {
            TurnFailureClass::Infrastructure => "infrastructure",
            TurnFailureClass::Timeout => "transient",
            _ => "product",
        };
        let truth = sample.label.as_str();
        // The rule has no "unknown" and no "transient" bucket of its own:
        // a failure it cannot place is a *default product* guess, which is
        // exactly the case that may be asked about.
        let ambiguous = predicted == "product" && truth != "product";
        if ambiguous {
            result.ambiguous += 1;
            if predicted == truth {
                result.ambiguous_correct += 1;
            }
        }
        if predicted == truth {
            result.correct += 1;
        }
        if predicted == truth {
            result
                .per_label
                .entry(truth.to_owned())
                .or_insert((0, 0, 0))
                .0 += 1;
        } else {
            result
                .per_label
                .entry(predicted.to_owned())
                .or_insert((0, 0, 0))
                .1 += 1;
            result
                .per_label
                .entry(truth.to_owned())
                .or_insert((0, 0, 0))
                .2 += 1;
        }
        // The dangerous direction: the rule says the environment is at
        // fault while the label says the product is.
        if predicted == "infrastructure" && truth == "product" {
            result.product_blocked += 1;
        }
    }
    if result.total == 0 {
        result
            .notes
            .push("the failure-triage corpus is missing".to_owned());
    }
    result
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
fn print_misses(samples: &[Sample], label: &str, limit: usize) {
    let mut shown = 0usize;
    for sample in samples {
        if shown >= limit {
            break;
        }
        let predicted = if label == "task" {
            classify_task_with_evidence(&sample.message)
                .baseline
                .label()
        } else {
            match classify_verify_failure(&sample.command_category, &sample.error_excerpt) {
                TurnFailureClass::Infrastructure => "infrastructure",
                TurnFailureClass::Timeout => "transient",
                _ => "product",
            }
        };
        if predicted == sample.label {
            continue;
        }
        shown += 1;
        let text: String = if label == "task" {
            sample.message.chars().take(40).collect()
        } else {
            sample.error_excerpt.chars().take(40).collect()
        };
        println!(
            "  {label} miss {}: predicted={predicted} truth={} \"{text}\"{}",
            sample.id,
            sample.label,
            if sample.note.is_empty() {
                String::new()
            } else {
                format!(" ({})", sample.note)
            }
        );
    }
}

/// Run the offline decision evaluation and print its report.
pub fn run() {
    println!("\nkodo-evals decision layer (offline, local rules only)");
    let task_samples = load("task_route.jsonl");
    let failure_samples = load("failure_triage.jsonl");
    let task = measure_task_route(&task_samples);
    let failure = measure_failure_triage(&failure_samples);

    println!("\n--- decision · task route (local rule baseline) ---");
    println!(
        "task_route_samples: {} ({} tune / {} holdout)",
        task.total,
        task_samples.iter().filter(|s| s.split == "tune").count(),
        task_samples.iter().filter(|s| s.split == "holdout").count(),
    );
    println!("task_route_accuracy: {:.3}", task.accuracy());
    println!("task_route_macro_f1: {:.3}", task.macro_f1());
    println!(
        "task_route_ambiguous: {} ({:.1}% of the corpus)",
        task.ambiguous,
        task.ambiguous_rate() * 100.0
    );
    println!(
        "task_route_ambiguous_correct: {} ({:.1}% of the unsure cases)",
        task.ambiguous_correct,
        if task.ambiguous == 0 {
            0.0
        } else {
            task.ambiguous_correct as f64 / task.ambiguous as f64 * 100.0
        }
    );
    for label in &task.labels {
        let (tp, fp, fn_) = task.per_label.get(label).copied().unwrap_or((0, 0, 0));
        println!("  task_route[{label}]: tp={tp} fp={fp} fn={fn_}");
    }
    println!("task_route_margin_sweep (ambiguous samples per margin):");
    for margin in [0u32, 2, 4, 8, 12] {
        let count = ambiguous_by_margin(&task_samples, margin);
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
    println!("failure_triage_samples: {}", failure.total);
    println!("failure_triage_accuracy: {:.3}", failure.accuracy());
    println!("failure_triage_macro_f1: {:.3}", failure.macro_f1());
    println!(
        "failure_triage_ambiguous: {} ({:.1}% of the corpus)",
        failure.ambiguous,
        failure.ambiguous_rate() * 100.0
    );
    println!(
        "failure_triage_product_blocked: {} ({:.1}% — a product failure the local rule would treat as an environment failure)",
        failure.product_blocked,
        if failure.total == 0 {
            0.0
        } else {
            failure.product_blocked as f64 / failure.total as f64 * 100.0
        }
    );
    for label in &failure.labels {
        let (tp, fp, fn_) = failure.per_label.get(label).copied().unwrap_or((0, 0, 0));
        println!("  failure_triage[{label}]: tp={tp} fp={fp} fn={fn_}");
    }

    println!("task_route_misses (first 10):");
    print_misses(&task_samples, "task", 10);
    println!("failure_triage_misses (first 10):");
    print_misses(&failure_samples, "failure", 10);

    println!("\n--- decision · offline contract check ---");
    let (mut contract_ok, mut contract_total) = check_state_contract(&task_samples);
    let (failure_ok, failure_total) = check_state_contract(&failure_samples);
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
    fn a_guessing_rule_scores_low() {
        // The local rule has no "unknown" bucket: a request it cannot place
        // falls back to `feature`, so an "unknown" sample is always a miss.
        let samples = vec![
            sample("新增导出", "feature"),
            sample("把那个东西弄好", "unknown"),
        ];
        let result = measure_task_route(&samples);
        assert_eq!(result.correct, 1);
        assert_eq!(result.accuracy(), 0.5);
    }

    #[test]
    fn the_ambiguous_subset_is_counted() {
        let samples = vec![sample("把那个东西弄好", "unknown")];
        let result = measure_task_route(&samples);
        assert_eq!(result.ambiguous, 1);
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
}
