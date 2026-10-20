//! Push-style streaming (sans-IO): feed chunks, take values as they
//! complete. Suits callers without a blocking reader (async sockets,
//! embedded DMA buffers).
//!
//! Decoding restarts from the start of the current top-level value when
//! a chunk runs out mid-value: O(n) per attempt, so a value
//! split across k chunks costs O(k·n). Fine for NDJSON-sized values;
//! for one huge document prefer `from_reader`.

use crate::reader::JsonReader;
use crate::source::PushSource;
use girders::{DecodeContext, DecodeError, DecodeOptions, Decoder, Reader, Span};

/// Decodes a sequence of whitespace-separated top-level values (NDJSON
/// and friends) from pushed chunks.
pub struct StreamDecoder<C> {
    codec: C,
    options: DecodeOptions,
    reader: JsonReader<PushSource>,
    /// Decode attempts that hit `Incomplete` and were retried (for
    /// measuring the restart cost).
    pub restarts: u64,
}

impl<C: Decoder> StreamDecoder<C> {
    pub fn new(codec: C) -> Self {
        Self::with_options(codec, DecodeOptions::new())
    }

    pub fn with_options(codec: C, options: DecodeOptions) -> Self {
        let mut reader = JsonReader::new(PushSource::new());
        reader.set_track_locations(options.track_locations);
        StreamDecoder {
            codec,
            options,
            reader,
            restarts: 0,
        }
    }

    /// Append a chunk of input.
    pub fn push(&mut self, chunk: &[u8]) {
        self.reader.source_mut().push(chunk);
    }

    /// Mark the end of input.
    pub fn finish(&mut self) {
        self.reader.source_mut().finish();
    }

    /// The next complete value, `None` if more input is needed (or the
    /// stream is finished), or the error. After an error the stream
    /// cannot continue.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<Result<C::Output, DecodeError<Span>>> {
        match self.reader.at_end() {
            Ok(true) => return None,
            Ok(false) => {}
            Err(e) if e.is_incomplete() => return None,
            Err(e) => return Some(Err(e)),
        }
        let cp = self.reader.checkpoint();
        let mut cx = DecodeContext::new(self.options);
        match self
            .codec
            .decode::<crate::DynReader<'_>>(&mut self.reader, &mut cx)
        {
            Ok(v) => {
                self.reader.release(cp);
                Some(Ok(v))
            }
            Err(e) if e.is_incomplete() => {
                self.reader.rewind(&cp);
                self.reader.release(cp);
                self.restarts += 1;
                None
            }
            Err(e) => {
                self.reader.release(cp);
                Some(Err(e))
            }
        }
    }
}
