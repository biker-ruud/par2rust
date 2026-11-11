//  This file is part of par2rust (a PAR 2.0 compatible file verification and
//  repair tool). See http://parchive.sourceforge.net for details of PAR 2.0.
//
//  Copyright (c) 2026 Michael Nightingale
//
//  par2rust is free software; you can redistribute it and/or modify
//  it under the terms of the GNU General Public License as published by
//  the Free Software Foundation; either version 2 of the License, or
//  (at your option) any later version.
//
//  par2rust is distributed in the hope that it will be useful,
//  but WITHOUT ANY WARRANTY; without even the implied warranty of
//  MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
//  GNU General Public License for more details.
//
//  You should have received a copy of the GNU General Public License
//  along with this program; if not, write to the Free Software
//  Foundation, Inc., 59 Temple Place, Suite 330, Boston, MA  02111-1307  USA

//! The implementations an application supplies. A factory left empty selects
//! the one built into par2rust.

use std::sync::Arc;

use crate::galois::Spec16;
use crate::hasher::{Hasher, ReferenceHasher};
use crate::processor::Processor;
use crate::reedsolomon::ReedSolomon;
use crate::reference_processor::ReferenceProcessor;

/// The budgets a backend is built with. What it accumulates is given
/// separately, once the number of recovery blocks is known.
#[derive(Clone, Copy, Default)]
pub struct ProcessorConfig {
    /// The thread budget, from -t
    pub numthreads: u32,
    /// The memory budget, from -m
    pub memorylimit: usize,
}

/// What a hasher is built with. Empty for now, and here so that it can gain a
/// field without every factory changing shape. It carries no thread budget,
/// because one instance belongs to one thread at a time and the caller keeps
/// the threading.
#[derive(Clone, Copy, Default)]
pub struct HasherConfig {}

/// A factory building the implementation of a [`Hasher`].
pub type HasherFactory = fn(&HasherConfig) -> Box<dyn Hasher>;

fn default_hasher(_config: &HasherConfig) -> Box<dyn Hasher> {
    Box::new(ReferenceHasher::new())
}

/// A factory building the implementation of a [`Processor`], given the
/// budgets and the Reed Solomon matrix it multiplies by.
pub type ProcessorFactory =
    fn(&ProcessorConfig, Arc<ReedSolomon<Spec16>>) -> Box<dyn Processor>;

fn default_processor(
    config: &ProcessorConfig,
    rs: Arc<ReedSolomon<Spec16>>,
) -> Box<dyn Processor> {
    Box::new(ReferenceProcessor::new(rs, config.numthreads))
}

#[derive(Default)]
pub struct Backends {
    /// A factory left empty selects the hasher built into par2rust.
    pub hasher: Option<HasherFactory>,
    /// A factory left empty selects the processor built into par2rust.
    pub processor: Option<ProcessorFactory>,
}

impl Backends {
    /// Build a hasher, taking the built-in reference one where no factory
    /// was supplied.
    pub fn build_hasher(&self) -> Box<dyn Hasher> {
        let factory = self.hasher.unwrap_or(default_hasher);
        factory(&HasherConfig::default())
    }

    /// Build a processor, taking the built-in reference one where no factory
    /// was supplied.
    pub fn build_processor(
        &self,
        config: &ProcessorConfig,
        rs: Arc<ReedSolomon<Spec16>>,
    ) -> Box<dyn Processor> {
        let factory = self.processor.unwrap_or(default_processor);
        factory(config, rs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_backends_build_reference_hasher() {
        let backends = Backends::default();
        let mut hasher = backends.build_hasher();
        assert!(hasher.init(100, 16, true));
        assert!(!hasher.init(100, 0, true));
    }

    #[test]
    fn factory_is_used_when_given() {
        fn factory(_config: &HasherConfig) -> Box<dyn Hasher> {
            Box::new(ReferenceHasher::new())
        }
        let backends = Backends {
            hasher: Some(factory),
            ..Default::default()
        };
        let mut hasher = backends.build_hasher();
        assert!(hasher.init(1, 1, false));
    }

    #[test]
    fn empty_backends_build_reference_processor() {
        let backends = Backends::default();
        let config = ProcessorConfig {
            numthreads: 2,
            memorylimit: 0,
        };
        let mut processor = backends.build_processor(&config, Arc::new(ReedSolomon::new()));
        assert!(processor.init(16, 3));
        processor.set_chunk_length(8);
        processor.reset_output();

        let factors = [1u16, 2, 3];
        let data: Vec<u8> = (0..8u8).collect();
        let ready = processor.add_input(&data, 0, Some(&factors));
        processor.end_input();
        ready
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the submission finished");

        let mut out = vec![0u8; 8];
        assert!(processor.get_output(0, &mut out));
        assert_eq!(&out, &data);
        assert!(processor.peek_output(1).is_some());
    }
}
