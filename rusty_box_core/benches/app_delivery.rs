use std::cell::OnceCell;
use std::convert::Infallible;
use std::hint::black_box;
use std::time::Instant;

use rusty_box_core::events::{App, ExecutionRequest, FromEvent};

const ROUNDS: usize = 9;
const EVENTS: u64 = 1_000_000;
const SEQUENCE_SUM: u64 = EVENTS * (EVENTS - 1) / 2;
const REPORT_WORDS: &str = "00000055 08049000 00000034 b7fa1000 00000040 00000008";

struct Tick(u64);
struct Read;
struct Write;
struct Halt;

struct Report {
    sequence: u64,
    words: &'static str,
    cache: OnceCell<Decoded>,
}

struct Decoded(u64);

impl FromEvent<Report> for Decoded {
    type Error = Infallible;

    fn from_event<'a>(event: &'a Report, _: &'a mut ()) -> Result<&'a Self, Self::Error> {
        Ok(event.cache.get_or_init(|| decode(event.words)))
    }
}

#[inline(never)]
fn decode(words: &str) -> Decoded {
    let total = words
        .split_ascii_whitespace()
        .map(|word| u64::from_str_radix(word, 16).expect("hex benchmark fixture"));
    Decoded(total.sum())
}

fn count_tick(tick: &Tick, total: &mut u64) {
    *total += tick.0;
}

fn count_report(report: &Report, total: &mut u64) {
    *total += report.sequence;
}

fn count_decoded(_: &Report, decoded: &Decoded, total: &mut u64) {
    *total += decoded.0;
}

fn report(sequence: u64) -> Report {
    Report {
        sequence,
        words: black_box(REPORT_WORDS),
        cache: OnceCell::new(),
    }
}

fn events<E>(
    mut dispatch: impl FnMut(&E) -> Result<ExecutionRequest, Infallible>,
    input: impl Fn(u64) -> E,
) -> f64 {
    let start = Instant::now();
    for sequence in 0..EVENTS {
        match dispatch(black_box(&input(black_box(sequence)))) {
            Ok(ExecutionRequest::None) => {}
            result => panic!("unexpected event delivery {result:?}"),
        }
    }
    start.elapsed().as_secs_f64() * 1e9 / EVENTS as f64
}

fn one_route() -> f64 {
    let mut app = App::new().route(count_tick).with_state(0);
    let ns = events(|tick| app.dispatch(tick), Tick);
    assert_eq!(black_box(app.into_state()), SEQUENCE_SUM);
    ns
}

fn unrelated_routes() -> f64 {
    let mut app = App::new()
        .route(|_: &Read| black_box(()))
        .route(|_: &Write| black_box(()))
        .route(count_tick)
        .route(|_: &Halt| black_box(()))
        .with_state(0);
    let ns = events(|tick| app.dispatch(tick), Tick);
    assert_eq!(black_box(app.into_state()), SEQUENCE_SUM);
    ns
}

fn unrequested_extraction() -> f64 {
    let mut app = App::new().route(count_report).with_state(0);
    let ns = events(|event| app.dispatch(event), report);
    assert_eq!(black_box(app.into_state()), SEQUENCE_SUM);
    ns
}

fn one_extraction() -> f64 {
    let mut app = App::new()
        .route(count_report)
        .route(count_decoded)
        .with_state(0);
    let ns = events(|event| app.dispatch(event), report);
    assert_eq!(
        black_box(app.into_state()),
        SEQUENCE_SUM + EVENTS * decode(REPORT_WORDS).0
    );
    ns
}

fn shared_extraction() -> f64 {
    let mut app = App::new()
        .route(count_report)
        .route(count_decoded)
        .route(count_decoded)
        .with_state(0);
    let ns = events(|event| app.dispatch(event), report);
    assert_eq!(
        black_box(app.into_state()),
        SEQUENCE_SUM + 2 * EVENTS * decode(REPORT_WORDS).0
    );
    ns
}

fn main() {
    let cases: [(&str, fn() -> f64); 5] = [
        ("one route", one_route),
        ("one plus three unrelated routes", unrelated_routes),
        ("extraction unrequested (ns/event)", unrequested_extraction),
        ("one extractor handler (ns/event)", one_extraction),
        ("two sharing extraction (ns/event)", shared_extraction),
    ];
    let mut times = [[0.0; ROUNDS]; 5];
    for (_, run) in cases {
        black_box(run());
    }
    for round in 0..ROUNDS {
        for offset in 0..cases.len() {
            let index = (round + offset) % cases.len();
            times[index][round] = cases[index].1();
        }
    }
    println!("Median ns/event over {ROUNDS} samples [min..max]");
    for ((name, _), mut values) in cases.into_iter().zip(times) {
        values.sort_unstable_by(f64::total_cmp);
        println!(
            "{name:34} {:7.3} [{:.3}..{:.3}]",
            values[ROUNDS / 2],
            values[0],
            values[ROUNDS - 1]
        );
    }
}
