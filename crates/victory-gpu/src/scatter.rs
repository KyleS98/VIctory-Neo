//! Small changed ranges of copies in the card's memory, sent up in one
//! pass (`VICTORY_CARD_SCATTER=1`; off by default: a copy for each, as
//! before).
//!
//! The title writes a few bytes at a time into its big buffers: 11,000 and
//! more changed runs a frame in the prologue, 60 KB in all. Each changed
//! range went to the card as a `CopyBufferRegion` of its own, and the
//! merge of title writes under flights (`Shadow::merge_under_writes`)
//! stopped at 512 of them and settled the flights instead: 25-35 ms of the
//! submitting thread a frame, waiting for the card. Here the small ranges
//! of a copy are dwords with a byte mask in an upload buffer, and one
//! dispatch puts them in: a dword is read, its masked bytes replaced and
//! written (the pass runs after the work before it on the queue, and
//! nothing else writes the copy meanwhile). Large ranges keep their copy.

use std::sync::{Mutex, OnceLock};

use windows::Win32::Graphics::Direct3D12::*;

use crate::host::{Host, compile};

/// Ranges this long or shorter go through the pass.
pub(crate) const SMALL: usize = 256;

const SOURCE: &str = r#"
#define RS "RootConstants(num32BitConstants = 1, b0), UAV(u0), SRV(t0)"
RWByteAddressBuffer target : register(u0);
ByteAddressBuffer entries : register(t0);
cbuffer Count : register(b0) { uint count; };
[RootSignature(RS)]
[numthreads(64, 1, 1)]
void scatter(uint id : SV_DispatchThreadID) {
    if (id >= count) {
        return;
    }
    uint3 e = entries.Load3(id * 12);
    if (e.z == 0xffffffff) {
        target.Store(e.x, e.y);
    } else {
        uint old = target.Load(e.x);
        target.Store(e.x, (old & ~e.z) | (e.y & e.z));
    }
}
"#;

pub(crate) struct Pass {
    pub(crate) root: ID3D12RootSignature,
    pub(crate) pso: ID3D12PipelineState,
}
// SAFETY: D3D12 objects are free-threaded.
unsafe impl Send for Pass {}
// SAFETY: as above.
unsafe impl Sync for Pass {}

pub(crate) fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_CARD_SCATTER").is_some_and(|v| v == "1"))
}

/// The pass, made when first needed (`None`: it could not be).
pub(crate) fn pass(device: &ID3D12Device) -> Option<&'static Pass> {
    static PASS: OnceLock<Option<Pass>> = OnceLock::new();
    PASS.get_or_init(|| {
        let made = (|| -> windows::core::Result<Pass> {
            let cs = compile(SOURCE, windows::core::s!("scatter"), windows::core::s!("cs_5_1"))?;
            // SAFETY: the root signature is inside the shader; the
            // description points at live data.
            unsafe {
                let root: ID3D12RootSignature = device.CreateRootSignature(0, &cs[..])?;
                let pso = device.CreateComputePipelineState(&D3D12_COMPUTE_PIPELINE_STATE_DESC {
                    pRootSignature: std::mem::transmute_copy(&root),
                    CS: D3D12_SHADER_BYTECODE {
                        pShaderBytecode: cs.as_ptr().cast(),
                        BytecodeLength: cs.len(),
                    },
                    ..Default::default()
                })?;
                Ok(Pass { root, pso })
            }
        })();
        match made {
            Ok(p) => {
                tracing::info!("small changed ranges of card copies go up in one pass (VICTORY_CARD_SCATTER=0: a copy each)");
                Some(p)
            }
            Err(e) => {
                tracing::warn!(error = %e, "no pass for small changed ranges of card copies; a copy each");
                None
            }
        }
    })
    .as_ref()
}

/// An upload buffer of entries, held by a plan until its submission ran.
pub(crate) struct Slot {
    pub(crate) buffer: ID3D12Resource,
    data: *mut u8,
    size: u64,
    /// Bytes used so far.
    pub(crate) used: u64,
}
// SAFETY: the mapping is ours; only the holder writes it.
unsafe impl Send for Slot {}

static POOL: Mutex<Vec<Slot>> = Mutex::new(Vec::new());

/// A slot of at least `size` bytes, from the pool or new.
pub(crate) fn take(host: &Host, size: u64) -> Option<Slot> {
    let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(i) = pool.iter().position(|s| s.size >= size) {
        let mut slot = pool.swap_remove(i);
        slot.used = 0;
        return Some(slot);
    }
    drop(pool);
    let size = size.next_power_of_two().max(1 << 20);
    let (buffer, data) = host.upload_buffer(size)?;
    Some(Slot { buffer, data, size, used: 0 })
}

/// Back to the pool: its submission has run.
pub(crate) fn give(slot: Slot) {
    let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
    if pool.len() < 64 {
        pool.push(slot);
    }
}

impl Slot {
    /// Writes `entries` at the slot's end; their GPU address, or `None` if
    /// they do not fit.
    pub(crate) fn put(&mut self, entries: &[[u32; 3]]) -> Option<u64> {
        let bytes = std::mem::size_of_val(entries) as u64;
        let at = self.used.next_multiple_of(16);
        if at + bytes > self.size {
            return None;
        }
        // SAFETY: inside the mapping (checked above), which only the holder
        // writes; the card reads it only after the submission is sent.
        unsafe {
            std::ptr::copy_nonoverlapping(entries.as_ptr().cast::<u8>(), self.data.add(at as usize), bytes as usize);
            self.used = at + bytes;
            Some(self.buffer.GetGPUVirtualAddress() + at)
        }
    }
}

/// The entries for `ranges` of a copy whose CPU side is `cpu` (`width`
/// bytes on the card): a dword each, with the mask of its bytes in the
/// ranges; a dword two ranges share is one entry. `false` if a range
/// reaches past the last whole dword (the caller copies those).
pub(crate) fn entries(cpu: *const u8, width: u64, ranges: &[(usize, usize)], out: &mut Vec<[u32; 3]>) -> bool {
    for &(_, b) in ranges {
        if b as u64 > width / 4 * 4 || b as u64 > u64::from(u32::MAX) {
            return false;
        }
    }
    for &(a, b) in ranges {
        let mut d = a / 4 * 4;
        while d < b {
            let mut mask = 0u32;
            for k in 0..4 {
                let at = d + k;
                if at >= a && at < b {
                    mask |= 0xff << (8 * k);
                }
            }
            // SAFETY: `d + 4 <= width`, inside the CPU side's mapping.
            let value = unsafe { cpu.add(d).cast::<u32>().read_unaligned() };
            match out.last_mut() {
                Some(last) if last[0] == d as u32 => {
                    last[1] = value;
                    last[2] |= mask;
                }
                _ => out.push([d as u32, value, mask]),
            }
            d += 4;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_mask_the_bytes_of_each_range_and_share_dwords() {
        let bytes: Vec<u8> = (0..32u8).collect();
        let mut out = Vec::new();
        assert!(entries(bytes.as_ptr(), 32, &[(1, 3), (3, 6), (12, 16)], &mut out));
        assert_eq!(
            out,
            [
                [0, u32::from_le_bytes([0, 1, 2, 3]), 0xffff_ff00],
                [4, u32::from_le_bytes([4, 5, 6, 7]), 0x0000_ffff],
                [12, u32::from_le_bytes([12, 13, 14, 15]), 0xffff_ffff],
            ]
        );
        let mut out = Vec::new();
        assert!(!entries(bytes.as_ptr(), 30, &[(28, 30)], &mut out));
    }
}
