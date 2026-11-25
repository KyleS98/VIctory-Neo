//! Native programs a title hands the device directly (device method 0x218
//! for compute, 0x1c8 for graphics) instead of as a shader container: code
//! already in title memory, its registers and a resource table. They are
//! repacked here as the container the console compiler would have made, so
//! they go through the same translation as every other program.
//!
//! Layout of a compute description, as the title passes it (0x60 bytes,
//! pointers into title memory):
//!
//! ```text
//! +0x00 root signature (the device's own object)
//! +0x08 registers, +0x10 their size (0x1c): u32 x2 ?, u32 0,
//!       u16 x, u16 y, u32 z (thread-group size), COMPUTE_PGM_RSRC1,
//!       COMPUTE_PGM_RSRC2
//! +0x18 header, +0x20 its size (0x14): u32 hash, u32 3, 0 ...
//! +0x28 GFX10 code, +0x30 its size (ends in s_endpgm)
//! +0x38 resource table, +0x40 its size
//! +0x48, +0x50 pointers (not read), +0x58 u64 hash
//! ```
//!
//! Resource table: a header (u32 kind: 3 compute, 2 a graphics stage; u32
//! size of the rest; u32 descriptor-table bytes; compute has two zero words
//! more), then tagged items:
//!
//! ```text
//! 0x40  u8 tag, u8 ?, u8 count of 0x41 items, u8 0 (4 bytes)
//! 0x41  u32 tag, the 20-byte used-resource entry of the FSSM record
//!       (`native.rs`), u32 -1, u32 0 (32 bytes)
//! 0x42  u32 tag, u32 0, u8 1, u8 class (0 SRV, 1 UAV, 2 CBV), u8 DXIL
//!       resource kind, u8 0, u32 register, u32 count, u32 0, u32 -1, u32 0
//! 0x43  u32 tag, 16-byte hash (the one the code's first v_nop carries)
//! ```

use crate::ShaderError::{self, Malformed};
use crate::dxbc::{FourCC, u32_at};
use crate::native::{
    ComputeIo, NativeProgram, ResourceType, RootParam, RootType, StageIo, UsedResource,
};

const TAG_TABLE: u8 = 0x40;
const TAG_USED: u32 = 0x41;
const ITEM: usize = 32;

/// The resources a resource table lists.
pub fn used_resources(table: &[u8]) -> Result<Vec<UsedResource>, ShaderError> {
    // The header is 0x0c or 0x14 bytes: the table item is the first word
    // tagged 0x40 that a used item (or nothing) follows.
    let at = (0x0c..=0x14)
        .step_by(4)
        .find(|&at| table.get(at) == Some(&TAG_TABLE))
        .ok_or(Malformed("prebuilt resource table without its 0x40 item"))?;
    let count = usize::from(table[at + 2]);
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let item = at + 4 + i * ITEM;
        if u32_at(table, item)? != TAG_USED {
            return Err(Malformed("prebuilt resource table: expected a used-resource item"));
        }
        let e = table
            .get(item + 4..item + 24)
            .ok_or(Malformed("prebuilt resource table out of bounds"))?;
        out.push(UsedResource {
            kind: match e[0] {
                0 => ResourceType::Srv,
                1 => ResourceType::Sampler,
                2 => ResourceType::Cbv,
                3 => ResourceType::Uav,
                n => ResourceType::Other(n),
            },
            root: RootType::from_raw(e[1].into()),
            table_offset_dwords: le32(e, 4),
            lower_bound: le32(e, 8),
            stride: le32(e, 12),
            dwords: e[16],
            user_data: e[17],
            slot: e[18],
        });
    }
    Ok(out)
}

/// The root parameters the used resources sit in, one per parameter: what
/// the program's user SGPRs hold.
/// shortcut: parameters the program reads nothing through are left out
/// (the table lists none), and a root constant's size is that of its entry.
pub fn layout_of(used: &[UsedResource]) -> Vec<RootParam> {
    let mut out: Vec<RootParam> = Vec::new();
    for u in used {
        let sgprs = match u.root {
            RootType::ImmediateSampler => continue,
            RootType::Descriptor => 2,
            RootType::Constants => u32::from(u.dwords.max(1)),
            _ => 1,
        };
        if let Some(p) = out.iter_mut().find(|p| p.slot == u.slot && p.root == u.root) {
            p.sgprs = p.sgprs.max(sgprs);
            continue;
        }
        out.push(RootParam {
            root: u.root,
            table_offset_dwords: 0,
            sgprs,
            base_register: 0,
            user_data: u.user_data,
            slot: u.slot,
        });
    }
    out
}

/// First user-data dword of the root parameters (the console's ABI puts
/// its own values in the four before).
const FIRST_USER_DATA: u32 = 4;
const HEAP_DIRECTLY_INDEXED: u32 = 0x400;
const SAMPLER_HEAP_DIRECTLY_INDEXED: u32 = 0x800;

/// Where the console compiler puts a root signature's parameters: from
/// user-data dword 4 on, in order, a table as its 32-bit heap address, root
/// constants as themselves, a root view as its 64-bit address; then the
/// heaps' addresses when the signature lets programs index them. `root` is
/// a serialized root signature (an `RTS0` part, or a container holding one).
pub fn root_layout(root: &[u8]) -> Result<Vec<RootParam>, ShaderError> {
    let rts0 = if root.starts_with(b"DXBC") {
        let c = crate::Container::parse(root)?;
        c.part(FourCC::RTS0).ok_or(Malformed("root signature container without RTS0"))?.data
    } else {
        root
    };
    let count = u32_at(rts0, 4)?;
    let params = u32_at(rts0, 8)? as usize;
    let flags = u32_at(rts0, 20)?;
    if count > 64 {
        return Err(Malformed("implausible root parameter count"));
    }
    let mut out = Vec::new();
    let mut next = FIRST_USER_DATA;
    for i in 0..count as usize {
        let at = params + 12 * i;
        let kind = u32_at(rts0, at)?;
        let data = u32_at(rts0, at + 8)? as usize;
        let (root, sgprs, base_register) = match kind {
            // A table: its ranges' registers are not needed here.
            0 => (RootType::Table, 1, 0),
            1 => (RootType::Constants, u32_at(rts0, data + 8)?, u32_at(rts0, data)?),
            2..=4 => (RootType::Descriptor, 2, u32_at(rts0, data)?),
            _ => return Err(Malformed("unknown root parameter type")),
        };
        out.push(RootParam {
            root,
            table_offset_dwords: 0,
            sgprs,
            base_register,
            user_data: next as u8,
            slot: i as u8,
        });
        next += sgprs;
    }
    for (bit, root) in [
        (HEAP_DIRECTLY_INDEXED, RootType::ResourceHeap),
        (SAMPLER_HEAP_DIRECTLY_INDEXED, RootType::SamplerHeap),
    ] {
        if flags & bit != 0 {
            out.push(RootParam {
                root,
                table_offset_dwords: 0,
                sgprs: 1,
                base_register: 0,
                user_data: next as u8,
                slot: 0,
            });
            next += 1;
        }
    }
    if next > 255 {
        return Err(Malformed("implausible root signature size"));
    }
    Ok(out)
}

/// Whether the code runs as 64-lane waves: it moves or masks `exec` or
/// `vcc` as 64-bit values.
pub fn runs_wave64(code: &[u8]) -> bool {
    let Ok(list) = crate::gfx10::decode(code) else {
        return true;
    };
    list.iter().any(|i| {
        let text = i.text(true);
        (text.contains("exec") || text.contains("vcc")) && text.contains("_b64")
            || text.contains("saveexec_b64")
    })
}

/// A compute program from the title's registers (`registers` as the
/// description's 0x1c bytes), code and resource table.
/// `root` is the serialized root signature the description names: the
/// layout comes from it (checked against the resource table and the user
/// SGPR count), else from the resource table alone.
pub fn compute_program(
    registers: &[u8],
    code: &[u8],
    table: &[u8],
    root: Option<&[u8]>,
) -> Result<NativeProgram, ShaderError> {
    if registers.len() < 0x1c {
        return Err(Malformed("prebuilt compute registers too short"));
    }
    let xy = u32_at(registers, 0x0c)?;
    let threads = [xy & 0xFFFF, xy >> 16, u32_at(registers, 0x10)? & 0xFFFF];
    if threads.iter().any(|&t| t == 0 || t > 1024) || threads.iter().product::<u32>() > 1024 {
        return Err(Malformed("prebuilt compute: implausible thread-group size"));
    }
    let rsrc1 = u32_at(registers, 0x14)?;
    let rsrc2 = u32_at(registers, 0x18)?;
    let wave64 = runs_wave64(code);
    let granule = if wave64 { 4 } else { 8 };
    let vgprs = (((rsrc1 & 0x3F) + 1) * granule).min(256) as u16;
    let used = used_resources(table)?;
    let layout = match root {
        Some(root) => {
            let layout = root_layout(root)?;
            let user_sgprs = rsrc2 >> 1 & 0x1F;
            if layout.iter().any(|p| u32::from(p.user_data) + p.sgprs > user_sgprs) {
                return Err(Malformed("prebuilt: root parameters past the user SGPRs (spilled)"));
            }
            let listed = |u: &UsedResource| {
                u.root == RootType::ImmediateSampler
                    || layout.iter().any(|p| p.root == u.root && p.user_data == u.user_data && p.slot == u.slot)
                    || matches!(u.root, RootType::ResourceHeap | RootType::SamplerHeap)
                        && layout.iter().any(|p| p.root == u.root && p.user_data == u.user_data)
            };
            if !used.iter().all(listed) {
                return Err(Malformed("prebuilt: the root signature's layout disagrees with the resource table"));
            }
            layout
        }
        None => layout_of(&used),
    };
    Ok(NativeProgram {
        stage: crate::HwStage::Cs,
        wave64,
        vgprs,
        layout,
        used,
        io: StageIo::Cs(ComputeIo { rsrc2, threads }),
        code: code.to_vec(),
    })
}

/// The shader container the console compiler would have made for
/// `program` (compute only): a `DXIL` part holding just the native blob.
pub fn container(program: &NativeProgram) -> Result<Vec<u8>, ShaderError> {
    let StageIo::Cs(cs) = &program.io else {
        return Err(Malformed("prebuilt container: compute programs only"));
    };
    let fssm = crate::native::fssm_compute(program, cs);
    let xbch = crate::native::xbch(&fssm);
    let part = crate::native::xdat_part(&xbch);
    Ok(crate::dxbc::build(&[(FourCC::DXIL, &part)]))
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first compute program a Series run handed method 0x218: a blit
    /// (sample one array texture, store to another), 8x8 threads.
    const REGISTERS: [u32; 7] = [0x4005cf00, 0x6c090102, 0, 0x0008_0008, 1, 0x402c0343, 0xb8a];

    fn table() -> Vec<u8> {
        let words: [u32; 30] = [
            0x0403, 0x104, 0x60, 0, 0, 0x0004_0440, //
            0x41, 0x0401, 0, 3, 0, 0x000b_0004, u32::MAX, 0, //
            0x41, 0x0300, 0, 0x10a, 4, 0x0408, u32::MAX, 0, //
            0x41, 0x0303, 8, 0x17, 4, 0x0408, u32::MAX, 0,
        ];
        let mut out: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        // The fourth item: the constant buffer.
        for w in [0x41u32, 0x0302, 0x10, 0x1b, 0, 0x0404, u32::MAX, 0] {
            out.extend_from_slice(&w.to_le_bytes());
        }
        out
    }

    #[test]
    fn reads_the_resource_table() {
        let used = used_resources(&table()).unwrap();
        assert_eq!(used.len(), 4);
        assert_eq!(used[0].kind, ResourceType::Sampler);
        assert_eq!(used[0].root, RootType::ImmediateSampler);
        assert_eq!(used[1].kind, ResourceType::Srv);
        assert_eq!((used[1].root, used[1].user_data, used[1].dwords), (RootType::Table, 4, 8));
        assert_eq!((used[2].kind, used[2].table_offset_dwords), (ResourceType::Uav, 8));
        assert_eq!((used[3].kind, used[3].table_offset_dwords), (ResourceType::Cbv, 16));
        let layout = layout_of(&used);
        assert_eq!(layout.len(), 1);
        assert_eq!((layout[0].root, layout[0].user_data, layout[0].sgprs), (RootType::Table, 4, 1));
    }

    #[test]
    fn round_trips_through_a_container() {
        let regs: Vec<u8> = REGISTERS.iter().flat_map(|w| w.to_le_bytes()).collect();
        // s_endpgm
        let code = 0xBF81_0000u32.to_le_bytes();
        let p = compute_program(&regs, &code, &table(), None).unwrap();
        assert_eq!(p.io, StageIo::Cs(ComputeIo { rsrc2: 0xb8a, threads: [8, 8, 1] }));
        assert!(!p.wave64);
        let c = container(&p).unwrap();
        let parsed = crate::Container::parse(&c).unwrap();
        let back = crate::native_programs(parsed.part(FourCC::DXIL).unwrap().data).unwrap();
        assert_eq!(back, [p]);
    }
}
