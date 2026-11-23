use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    Stdout,
    Stderr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IoError {
    Interrupted,
    WouldBlock,
    Failure,
    NoSpace,
    BrokenPipe,
}

impl IoError {
    pub(crate) fn errno(self) -> i32 {
        match self {
            Self::Interrupted => 4,
            Self::Failure => 5,
            Self::WouldBlock => 11,
            Self::NoSpace => 28,
            Self::BrokenPipe => 32,
        }
    }
}

/// Application-owned standard streams. Return the number of bytes produced
/// or consumed, at most the supplied length. A read of zero signals EOF.
/// Errors report zero transferred bytes. Calls are synchronous.
pub trait Io {
    fn read_stdin(&mut self, buffer: &mut [u8]) -> Result<usize, IoError>;
    fn write(&mut self, stream: Output, buffer: &[u8]) -> Result<usize, IoError>;
}

/// Deterministic borrowed input and two owned output buffers. The output
/// limit applies independently to stdout and stderr.
pub struct BufferedIo<'a> {
    input: &'a [u8],
    offset: usize,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    limit: usize,
}

impl<'a> BufferedIo<'a> {
    pub fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            offset: 0,
            stdout: Vec::new(),
            stderr: Vec::new(),
            limit: 1024 * 1024,
        }
    }

    pub fn output_limit(mut self, bytes_per_stream: usize) -> Self {
        self.limit = bytes_per_stream;
        self
    }

    pub fn input_consumed(&self) -> usize {
        self.offset
    }

    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }
}

impl Io for BufferedIo<'_> {
    fn read_stdin(&mut self, buffer: &mut [u8]) -> Result<usize, IoError> {
        let len = buffer.len().min(self.input.len() - self.offset);
        buffer[..len].copy_from_slice(&self.input[self.offset..self.offset + len]);
        self.offset += len;
        Ok(len)
    }

    fn write(&mut self, stream: Output, buffer: &[u8]) -> Result<usize, IoError> {
        let out = match stream {
            Output::Stdout => &mut self.stdout,
            Output::Stderr => &mut self.stderr,
        };
        let len = buffer.len().min(self.limit.saturating_sub(out.len()));
        if len == 0 && !buffer.is_empty() {
            return Err(IoError::NoSpace);
        }
        out.try_reserve(len).map_err(|_| IoError::NoSpace)?;
        out.extend_from_slice(&buffer[..len]);
        Ok(len)
    }
}
