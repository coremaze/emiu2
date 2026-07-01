use super::wdc_65c02::{FetchedInstruction, Instruction};
use crate::memory::ContentChange;

/// Cache pages are 4 KiB of key space (matching the flash's erase sector
/// size); an entry exists per byte since instructions may start anywhere.
const PAGE_SIZE: usize = 4096;

/// Keys cover the machine's cacheable code storage (2 MiB flash + 16 KiB
/// OTP for the handheld). Keys at or above this are rejected rather than
/// silently aliased.
const KEY_SPACE: usize = 0x21_0000;

const NUM_PAGES: usize = KEY_SPACE.div_ceil(PAGE_SIZE);

/// A cache slot: a fetched instruction plus a validity flag
#[derive(Clone, Copy)]
struct CachedEntry {
    fetched: FetchedInstruction,
    valid: bool,
}

impl CachedEntry {
    const INVALID: CachedEntry = CachedEntry {
        fetched: FetchedInstruction {
            instruction: Instruction {
                opcode: super::wdc_65c02::Opcode::Nop,
                addressing_mode: super::wdc_65c02::AddressingMode::Implied,
            },
            cycles: 0,
            length: 0,
            extra_page_boundary_cycle: false,
        },
        valid: false,
    };
}

/// Cache for instructions in internal RAM, where self-modifying code is
/// routine (e.g. flash-programming loops patch their own operands). Each
/// entry remembers the raw bytes it was decoded from; a hit compares them
/// against RAM, so modifying an instruction invalidates exactly that
/// instruction and writes need no cache bookkeeping at all.
pub struct RamDecodeCache {
    pages: Vec<Option<Box<[RamCachedEntry]>>>,
}

#[derive(Clone, Copy)]
struct RamCachedEntry {
    entry: CachedEntry,
    bytes: [u8; 3],
}

const RAM_PAGE_SIZE: usize = 256;

impl RamDecodeCache {
    pub fn new(ram_len: usize) -> Self {
        let num_pages = ram_len.div_ceil(RAM_PAGE_SIZE);
        Self {
            pages: (0..num_pages).map(|_| None).collect(),
        }
    }

    /// Look up the entry at `ram_index`, verifying it against the current
    /// RAM contents. The caller guarantees the instruction does not cross a
    /// 256-byte page, so `ram_index + 2` is a valid index.
    #[inline]
    pub fn get(&self, ram_index: usize, ram: &[u8]) -> Option<FetchedInstruction> {
        let page = self.pages[ram_index / RAM_PAGE_SIZE].as_deref()?;
        let cached = page[ram_index % RAM_PAGE_SIZE];
        if !cached.entry.valid {
            return None;
        }
        // Manual compare: lengths are 1..=3, not worth a memcmp call
        for i in 0..cached.entry.fetched.length as usize {
            if ram[ram_index + i] != cached.bytes[i] {
                return None;
            }
        }
        Some(cached.entry.fetched)
    }

    pub fn insert(&mut self, ram_index: usize, fetched: FetchedInstruction, bytes: [u8; 3]) {
        const EMPTY: RamCachedEntry = RamCachedEntry {
            entry: CachedEntry::INVALID,
            bytes: [0; 3],
        };
        let page = self.pages[ram_index / RAM_PAGE_SIZE]
            .get_or_insert_with(|| vec![EMPTY; RAM_PAGE_SIZE].into_boxed_slice());
        page[ram_index % RAM_PAGE_SIZE] = RamCachedEntry {
            entry: CachedEntry {
                fetched,
                valid: true,
            },
            bytes,
        };
    }
}

/// Decoded-instruction cache keyed by the machine's code cache keys, which
/// are alias-free device-local addresses — bank switches never invalidate
/// anything, and content changes invalidate exactly the affected pages.
pub struct DecodeCache {
    pages: Vec<Option<Box<[CachedEntry]>>>,
}

impl DecodeCache {
    pub fn new() -> Self {
        Self {
            pages: (0..NUM_PAGES).map(|_| None).collect(),
        }
    }

    #[inline]
    pub fn get(&self, key: usize) -> Option<FetchedInstruction> {
        let page = self.pages.get(key / PAGE_SIZE)?.as_deref()?;
        let entry = page[key % PAGE_SIZE];
        entry.valid.then_some(entry.fetched)
    }

    pub fn insert(&mut self, key: usize, fetched: FetchedInstruction) {
        let Some(slot) = self.pages.get_mut(key / PAGE_SIZE) else {
            return;
        };

        let page =
            slot.get_or_insert_with(|| vec![CachedEntry::INVALID; PAGE_SIZE].into_boxed_slice());
        page[key % PAGE_SIZE] = CachedEntry {
            fetched,
            valid: true,
        };
    }

    /// Drop cached decodes covering changed contents. An instruction never
    /// spans a page boundary (such fetches are not cached), so dropping the
    /// pages overlapping the range is sufficient.
    pub fn apply_change(&mut self, change: ContentChange) {
        match change {
            ContentChange::None => {}
            ContentChange::Range { start, len } => {
                let first_page = start / PAGE_SIZE;
                let last_page = (start + len.max(1) - 1) / PAGE_SIZE;
                for page in first_page..=last_page {
                    if let Some(slot) = self.pages.get_mut(page) {
                        *slot = None;
                    }
                }
            }
            ContentChange::All => {
                for slot in &mut self.pages {
                    *slot = None;
                }
            }
        }
    }
}
