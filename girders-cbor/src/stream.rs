//! Push-style streaming (sans-IO) of CBOR sequences (RFC 8742): data
//! items one after another, fed in chunks as they arrive. Suits callers
//! without a blocking reader, such as async sockets or embedded DMA
//! buffers.
//!
//! When a chunk ends mid-item, decoding restarts from the start of that
//! item once more input arrives: an item split across k chunks costs
//! O(k·n). Fine for message-sized items; for one huge item prefer
//! `from_reader`.

use crate::reader::CborReader;
use girders::source::PushSource;
use girders::{DecodeContext, DecodeError, DecodeOptions, Decoder, Reader, Span};

/// Decodes a CBOR sequence from pushed chunks.
pub struct StreamDecoder<C> {
    codec: C,
    options: DecodeOptions,
    reader: CborReader<PushSource>,
    /// Decode attempts that ran out of input and were retried.
    pub restarts: u64,
}

impl<C: Decoder> StreamDecoder<C> {
    pub fn new(codec: C) -> Self {
        Self::with_options(codec, DecodeOptions::new())
    }

    pub fn with_options(codec: C, options: DecodeOptions) -> Self {
        let mut reader = CborReader::new(PushSource::new());
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

    /// The next complete item, `None` if more input is needed (or the
    /// stream is finished), or the error. After an error the stream cannot
    /// continue.
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
