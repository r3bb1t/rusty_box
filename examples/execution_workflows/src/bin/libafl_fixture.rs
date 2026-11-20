//! Exercise LibAFL coverage feedback and replay on a fixed benign fixture.

use libafl::Error;
use libafl::corpus::{Corpus, InMemoryCorpus};
use libafl::events::NopEventManager;
use libafl::feedbacks::{ConstFeedback, MaxMapFeedback};
use libafl::fuzzer::{Evaluator, StdFuzzer};
use libafl::inputs::BytesInput;
use libafl::schedulers::QueueScheduler;
use libafl::state::{HasCorpus, StdState};
use libafl_bolts::rands::StdRand;
use rusty_box_execution_workflows::fixture::{FixtureExecutor, Outcome, benchmark};

fn evaluate() -> Result<(), Error> {
    let mut executor = FixtureExecutor::new()?;
    let mut feedback = MaxMapFeedback::new(executor.coverage());
    let mut objective = ConstFeedback::False;
    let mut state = StdState::new(
        StdRand::with_seed(0),
        InMemoryCorpus::new(),
        InMemoryCorpus::new(),
        &mut feedback,
        &mut objective,
    )?;
    let mut fuzzer = StdFuzzer::new(QueueScheduler::new(), feedback, objective);
    let mut events = NopEventManager::new();

    for (byte, expected) in [
        (b'A', Outcome::Accepted),
        (b'?', Outcome::Rejected),
        (b'T', Outcome::DispatchBudget),
        (b'A', Outcome::Accepted),
    ] {
        let (_, retained) = fuzzer.evaluate_input(
            &mut state,
            &mut executor,
            &mut events,
            &BytesInput::new(vec![byte]),
        )?;
        let replay = executor
            .last()
            .ok_or_else(|| Error::illegal_state("fixture executed without producing a result"))?;
        assert_eq!(replay.run.status, expected);
        println!(
            "input={byte:02x} {replay:?} interesting={}",
            retained.is_some()
        );
    }
    assert_eq!(state.corpus().count(), 3);
    println!("LibAFL retained three distinct coverage paths.");
    Ok(())
}

fn main() -> Result<(), Error> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => evaluate(),
        [option] if option == "--bench" => benchmark(),
        [option, byte] if option == "--replay" => {
            let byte = u8::from_str_radix(byte, 16)
                .map_err(|_| Error::illegal_argument("expected a hex byte"))?;
            println!("{:?}", FixtureExecutor::new()?.replay(&[byte])?);
            Ok(())
        }
        _ => Err(Error::illegal_argument(
            "usage: libafl_fixture [--bench | --replay HEX_BYTE]",
        )),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn actual_libafl_feedback_retains_each_path_once() {
        super::evaluate().unwrap();
    }
}
