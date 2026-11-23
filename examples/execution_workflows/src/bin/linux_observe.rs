use std::fs;
use std::path::PathBuf;

use rusty_box::events::{App, AppHandlers, ExecutionRequest};
use rusty_box_execution_workflows::linux::AddressSpace;
use rusty_box_execution_workflows::linux::console::ConsoleResult;
use rusty_box_execution_workflows::linux::semantic::{
    Guest, ImageMetadata, ImageObserved, MetadataRequest, ProcessObserved,
};

#[derive(Default)]
struct State {
    processes: Vec<(u32, AddressSpace)>,
    images: Vec<u64>,
    inspected: usize,
    shared: usize,
}

mod processes {
    use super::*;

    pub(super) fn register<H: AppHandlers<State, String>>(
        app: App<State, String, H>,
    ) -> App<State, String, impl AppHandlers<State, String>> {
        app.on(|process: &ProcessObserved, state: &mut State| {
            state.processes.push((process.pid, process.space));
            println!(
                "Process {} pid={} cr3={:#x} generation={}",
                process.executable,
                process.pid,
                process.space.page_table_root,
                process.space.image_generation,
            );
        })
    }
}

mod images {
    use super::*;

    pub(super) fn register<H: AppHandlers<State, String>>(
        app: App<State, String, H>,
    ) -> App<State, String, impl AppHandlers<State, String>> {
        app.on(|image: &ImageObserved, state: &mut State| {
            if state.processes.len() != state.images.len() + 1
                || state.processes.last() != Some(&(image.pid, image.space))
            {
                return Err("image arrived outside its process observation".to_owned());
            }
            state.images.push(image.entry);
            Ok(ExecutionRequest::None)
        })
        .on(|image: &ImageObserved| {
            println!("Image pid={} entry={:#x}", image.pid, image.entry);
            for region in &image.executable_regions {
                println!(
                    "Executable mapping {:#x}..{:#x} file offset {:#x}",
                    region.start, region.end, region.file_offset
                );
            }
        })
    }
}

mod elf {
    use super::*;

    pub(super) fn register<H: AppHandlers<State, String>>(
        app: App<State, String, H>,
    ) -> App<State, String, impl AppHandlers<State, String>> {
        app.on(|metadata: &ImageMetadata, state: &mut State| {
            let elf = metadata
                .result
                .as_ref()
                .map_err(|error| error.to_string())?;
            if state.images.last() != Some(&elf.entry)
                || state.processes.last().map(|(_, space)| *space) != Some(metadata.space)
            {
                return Err("ELF metadata arrived outside its image observation".to_owned());
            }
            state.inspected += 1;
            println!(
                "Mapped ELF {:?} link entry={:#x} runtime entry={:#x} bias={:#x}",
                elf.executable_type, elf.link_entry, elf.entry, elf.load_bias,
            );
            Ok(ExecutionRequest::None)
        })
        .on(|metadata: &ImageMetadata, state: &mut State| {
            let elf = metadata
                .result
                .as_ref()
                .map_err(|error| error.to_string())?;
            if state.inspected != state.shared + 1 {
                return Err("metadata observers ran out of registration order".to_owned());
            }
            state.shared += 1;
            println!(
                "Shared ELF result with {} program headers",
                elf.program_headers.len()
            );
            Ok(ExecutionRequest::None)
        })
    }
}

fn run<H: AppHandlers<State, String>>(
    app: App<State, String, H>,
    mut guest: Guest,
    request: MetadataRequest,
) -> ConsoleResult<()> {
    let mut app = app.with_state(State::default());
    for (program, output) in [
        ("/bin/program_sum", "sum=22"),
        ("/bin/program_text", "argv0=/bin/program_text"),
    ] {
        let paused = guest.observe(program, request)?;
        match paused.observation().deliver(&mut app)? {
            ExecutionRequest::None => {}
            ExecutionRequest::Stop => return Err("observer requested a stop at user entry".into()),
        }
        let completed = paused.finish()?;
        if completed.wait_status != 0 || completed.output != [output] {
            return Err(format!(
                "{program} exited with wait status {} and output {:?}",
                completed.wait_status, completed.output
            )
            .into());
        }
        println!("Guest output {output}");
        guest = completed.guest;
    }
    let state = app.into_state();
    let inspected = if request == MetadataRequest::ReadMappedElf {
        2
    } else {
        0
    };
    if state.processes.len() != 2
        || state.images.len() != 2
        || state.processes[0].1 == state.processes[1].1
        || state.inspected != inspected
        || state.shared != inspected
    {
        return Err("unexpected recovered observation state".into());
    }
    println!("Recovered state with two processes, two images, and {inspected} ELF results");
    Ok(())
}

fn main() -> ConsoleResult<()> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let (kernel, initramfs, request) = match args.as_slice() {
        [kernel, initramfs] => (kernel, initramfs, MetadataRequest::Skip),
        [kernel, initramfs, flag] if flag.as_os_str() == "--elf" => {
            (kernel, initramfs, MetadataRequest::ReadMappedElf)
        }
        _ => return Err("usage: linux_observe BZIMAGE INITRAMFS [--elf]".into()),
    };
    let handlers = images::register(processes::register(App::<State, String>::new()));
    let guest = Guest::boot(&fs::read(kernel)?, &fs::read(initramfs)?)?;
    match request {
        MetadataRequest::Skip => run(handlers, guest, request),
        MetadataRequest::ReadMappedElf => run(elf::register(handlers), guest, request),
    }
}
