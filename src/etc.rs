//! ETC2 and EAC block compression, for textures the GPU reads compressed.
//!
//! The V3D decodes ETC2 and EAC in its texture unit on both GL (they are core
//! in OpenGL ES 3) and Vulkan, so a compressed map costs a quarter to an
//! eighth of the memory traffic of its RGBA8 original. The globe, whose frame
//! is mostly texture fetches from 4096-texel maps, is 0.4 ms faster for it.
//! The dive's surfaces were tried too and gained nothing measurable: 256
//! texels square, they already sit in the texture cache.
//!
//! The encoder is a plain exhaustive one, written for this project rather than
//! fast: `build.rs` compresses the Earth's maps with it, and the decoder
//! unpacks them again on a GPU without ETC2. The encoder writes the ETC1
//! modes (individual and differential), ETC2's planar mode, which keeps
//! smooth gradients such as the open ocean free of blocks, and EAC for one-
//! and two-channel maps. ETC2's T and H modes are left out.
//!
//! Blocks are 4 × 4 texels, row-major in the image and stored as their 64-bit
//! words, most significant byte first, as the formats define them. Images whose
//! sides are not multiples of four are padded by repeating their last row and
//! column, which the GPU never samples.

// `build.rs` includes this file as well, and uses only part of it.
#![allow(dead_code)]

/// ETC1's intensity modifiers, the small and the large step of each table.
const ETC1_TABLES: [[i32; 2]; 8] = [
    [2, 8],
    [5, 17],
    [9, 29],
    [13, 42],
    [18, 60],
    [24, 80],
    [33, 106],
    [47, 183],
];

/// EAC's modifier tables.
const EAC_TABLES: [[i32; 8]; 16] = [
    [-3, -6, -9, -15, 2, 5, 8, 14],
    [-3, -7, -10, -13, 2, 6, 9, 12],
    [-2, -5, -8, -13, 1, 4, 7, 12],
    [-2, -4, -6, -13, 1, 3, 5, 12],
    [-3, -6, -8, -12, 2, 5, 7, 11],
    [-3, -7, -9, -11, 2, 6, 8, 10],
    [-4, -7, -8, -11, 3, 6, 7, 10],
    [-3, -5, -8, -11, 2, 4, 7, 10],
    [-2, -6, -8, -10, 1, 5, 7, 9],
    [-2, -5, -8, -10, 1, 4, 7, 9],
    [-2, -4, -8, -10, 1, 3, 7, 9],
    [-2, -5, -7, -10, 1, 4, 6, 9],
    [-3, -4, -7, -10, 2, 3, 6, 9],
    [-1, -2, -3, -10, 0, 1, 2, 9],
    [-4, -6, -8, -9, 3, 5, 7, 8],
    [-3, -5, -7, -9, 2, 4, 6, 8],
];

/// The position of texel (x, y) in a block's index bits: the formats number
/// texels down each column first.
fn slot(x: usize, y: usize) -> usize {
    x * 4 + y
}

/// Expand a 4-, 5-, 6- or 7-bit colour component to 8 bits.
fn expand(value: i32, bits: u32) -> i32 {
    (value << (8 - bits)) | (value >> (2 * bits - 8))
}

fn square(v: i32) -> i32 {
    v * v
}

/// The best way to tint one half block from one base colour: its error, its
/// modifier table and each of its eight texels' index (as stored).
#[derive(Clone, Copy)]
struct Half {
    error: i32,
    table: u32,
    indices: [u32; 8],
}

/// Fit eight texels to `base` (8-bit) with each modifier table.
fn fit_half(texels: &[[i32; 3]; 8], base: [i32; 3], limit: i32) -> Half {
    let mut best = Half {
        error: i32::MAX,
        table: 0,
        indices: [0; 8],
    };
    for (table, steps) in ETC1_TABLES.iter().enumerate() {
        // Index 0 is +small, 1 +large, 2 -small, 3 -large.
        let modifiers = [steps[0], steps[1], -steps[0], -steps[1]];
        let mut error = 0;
        let mut indices = [0u32; 8];
        for (i, texel) in texels.iter().enumerate() {
            let mut texel_best = i32::MAX;
            for (index, m) in modifiers.iter().enumerate() {
                let e = square((base[0] + m).clamp(0, 255) - texel[0])
                    + square((base[1] + m).clamp(0, 255) - texel[1])
                    + square((base[2] + m).clamp(0, 255) - texel[2]);
                if e < texel_best {
                    texel_best = e;
                    indices[i] = index as u32;
                }
            }
            error += texel_best;
            if error >= best.error.min(limit) {
                break;
            }
        }
        if error < best.error {
            best = Half {
                error,
                table: table as u32,
                indices,
            };
        }
    }
    best
}

/// Base colours near a half block's mean in `bits` per component: every
/// rounding down or up of each component.
fn candidates(texels: &[[i32; 3]; 8], bits: u32) -> Vec<[i32; 3]> {
    let top = (1 << bits) - 1;
    let mean = [0, 1, 2].map(|c| texels.iter().map(|t| t[c]).sum::<i32>() as f32 / 8.0);
    let scaled = mean.map(|m| m * top as f32 / 255.0);
    let mut out = Vec::with_capacity(8);
    for corner in 0..8 {
        let colour = [0, 1, 2].map(|c| {
            let v = if corner >> c & 1 == 0 {
                scaled[c].floor()
            } else {
                scaled[c].ceil()
            };
            (v as i32).clamp(0, top)
        });
        if !out.contains(&colour) {
            out.push(colour);
        }
    }
    out
}

/// The texel positions of a half block: the left or right two columns
/// (`flip` false), or the top or bottom two rows.
fn half_positions(flip: bool, half: usize) -> [(usize, usize); 8] {
    std::array::from_fn(|i| {
        if flip {
            (i % 4, half * 2 + i / 4)
        } else {
            (half * 2 + i % 2, i / 2)
        }
    })
}

/// The two halves of a block, in [`half_positions`] order.
fn halves(block: &[[i32; 3]; 16], flip: bool) -> [[[i32; 3]; 8]; 2] {
    [0, 1].map(|half| half_positions(flip, half).map(|(x, y)| block[y * 4 + x]))
}

/// An encoded RGB block with its squared error.
struct Encoded {
    error: i32,
    word: u64,
}

/// The ETC1 modes, individual and differential, both ways of halving.
fn encode_etc1(block: &[[i32; 3]; 16]) -> Encoded {
    let mut best = Encoded {
        error: i32::MAX,
        word: 0,
    };
    for flip in [false, true] {
        let parts = halves(block, flip);
        // Individual: two 4-bit colours, each the best of its candidates.
        let fits = parts.map(|part| {
            let mut found = (
                Half {
                    error: i32::MAX,
                    table: 0,
                    indices: [0; 8],
                },
                [0; 3],
            );
            for colour in candidates(&part, 4) {
                let fit = fit_half(&part, colour.map(|c| expand(c, 4)), found.0.error);
                if fit.error < found.0.error {
                    found = (fit, colour);
                }
            }
            found
        });
        let total = fits[0].0.error + fits[1].0.error;
        if total < best.error {
            let mut word = 0u64;
            for c in 0..3 {
                word |= (fits[0].1[c] as u64) << (60 - 8 * c);
                word |= (fits[1].1[c] as u64) << (56 - 8 * c);
            }
            word |= etc1_tail(&fits, flip, false);
            best = Encoded { error: total, word };
        }

        // Differential: a 5-bit colour and a second within -4..3 of it.
        let options: [Vec<(Half, [i32; 3])>; 2] = [0, 1].map(|half| {
            candidates(&parts[half], 5)
                .into_iter()
                .map(|colour| {
                    (
                        fit_half(&parts[half], colour.map(|c| expand(c, 5)), i32::MAX),
                        colour,
                    )
                })
                .collect()
        });
        for first in &options[0] {
            for second in &options[1] {
                let fits_range = (0..3).all(|c| (-4..=3).contains(&(second.1[c] - first.1[c])));
                let error = first.0.error + second.0.error;
                if fits_range && error < best.error {
                    let mut word = 0u64;
                    for c in 0..3 {
                        let delta = (second.1[c] - first.1[c]) & 7;
                        word |= (first.1[c] as u64) << (59 - 8 * c);
                        word |= (delta as u64) << (56 - 8 * c);
                    }
                    word |= etc1_tail(&[*first, *second], flip, true);
                    best = Encoded { error, word };
                }
            }
        }
    }
    best
}

/// The low half of an ETC1 word: tables, the mode bits and the indices.
fn etc1_tail(fits: &[(Half, [i32; 3]); 2], flip: bool, differential: bool) -> u64 {
    let mut word = (fits[0].0.table as u64) << 37 | (fits[1].0.table as u64) << 34;
    word |= (differential as u64) << 33 | (flip as u64) << 32;
    for (half, (fit, _)) in fits.iter().enumerate() {
        for (i, (x, y)) in half_positions(flip, half).into_iter().enumerate() {
            let s = slot(x, y);
            let index = fit.indices[i] as u64;
            word |= (index >> 1) << (16 + s) | (index & 1) << s;
        }
    }
    word
}

/// ETC2's planar mode: three corner colours the block's texels are a linear
/// blend of, fitted by least squares.
fn encode_planar(block: &[[i32; 3]; 16]) -> Encoded {
    // c(x, y) = O (1 - x/4 - y/4) + H x/4 + V y/4: the normal equations
    // are the same for every block and channel.
    let weights = |x: usize, y: usize| {
        let (x, y) = (x as f32 / 4.0, y as f32 / 4.0);
        [1.0 - x - y, x, y]
    };
    let mut normal = [[0.0f32; 3]; 3];
    let mut right = [[0.0f32; 3]; 3];
    for y in 0..4 {
        for x in 0..4 {
            let w = weights(x, y);
            for i in 0..3 {
                for j in 0..3 {
                    normal[i][j] += w[i] * w[j];
                }
                for c in 0..3 {
                    right[c][i] += w[i] * block[y * 4 + x][c] as f32;
                }
            }
        }
    }
    let solved = right.map(|b| solve3(normal, b));
    // Quantised to 6, 7 and 6 bits a component: O, H and V each.
    let bits = [6, 7, 6];
    let quantised: [[i32; 3]; 3] = std::array::from_fn(|corner| {
        std::array::from_fn(|c| {
            let top = (1 << bits[c]) - 1;
            ((solved[c][corner] * top as f32 / 255.0).round() as i32).clamp(0, top)
        })
    });
    let colour = quantised.map(|corner| [0, 1, 2].map(|c| expand(corner[c], bits[c])));
    let mut error = 0;
    for y in 0..4 {
        for x in 0..4 {
            for c in 0..3 {
                let [o, h, v] = [colour[0][c], colour[1][c], colour[2][c]];
                let value =
                    ((x as i32 * (h - o) + y as i32 * (v - o) + 4 * o + 2) >> 2).clamp(0, 255);
                error += square(value - block[y * 4 + x][c]);
            }
        }
    }
    let [o, h, v] = quantised.map(|c| c.map(|v| v as u64));
    let mut word = o[0] << 57
        | (o[1] >> 6) << 56
        | (o[1] & 0x3f) << 49
        | (o[2] >> 5) << 48
        | (o[2] >> 3 & 3) << 43
        | (o[2] & 7) << 39
        | (h[0] >> 1) << 34
        | 1 << 33
        | (h[0] & 1) << 32
        | h[1] << 25
        | h[2] << 19
        | v[0] << 13
        | v[1] << 6
        | v[2];
    // The bits the colours leave free (63, 55, 47-45, 42) are set so that a
    // decoder reading the word as a differential block finds red and green
    // in range and blue out of it, which is what marks planar mode.
    let free = [63, 55, 47, 46, 45, 42];
    let in_range = |word: u64, at: u32| {
        let base = (word >> (at + 3) & 31) as i32;
        let delta = ((word >> at & 7) as i32 ^ 4) - 4;
        (0..=31).contains(&(base + delta))
    };
    let filler = (0..1u64 << free.len())
        .map(|pattern| {
            free.iter()
                .enumerate()
                .fold(0, |acc, (i, bit)| acc | (pattern >> i & 1) << bit)
        })
        .find(|filler| {
            let w = word | filler;
            in_range(w, 56) && in_range(w, 48) && !in_range(w, 40)
        })
        .expect("a planar block always has a filler");
    word |= filler;
    Encoded { error, word }
}

/// Solve a symmetric 3 × 3 system by Cramer's rule.
fn solve3(a: [[f32; 3]; 3], b: [f32; 3]) -> [f32; 3] {
    let det = |m: [[f32; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let d = det(a);
    std::array::from_fn(|k| {
        let mut m = a;
        for row in 0..3 {
            m[row][k] = b[row];
        }
        det(m) / d
    })
}

/// One ETC2 RGB block from 16 texels, row-major; alpha is ignored.
pub fn rgb_block(texels: &[[u8; 4]; 16]) -> [u8; 8] {
    let block = texels.map(|t| [t[0] as i32, t[1] as i32, t[2] as i32]);
    let mut best = encode_etc1(&block);
    if best.error > 0 {
        let planar = encode_planar(&block);
        if planar.error < best.error {
            best = planar;
        }
    }
    best.word.to_be_bytes()
}

/// How an EAC block is decoded: as ETC2's 8-bit alpha, or as an 11-bit
/// channel of the R11 and RG11 formats.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Eac {
    Alpha8,
    R11,
}

/// One EAC block from 16 values, row-major.
pub fn eac_block(values: &[u8; 16], kind: Eac) -> [u8; 8] {
    // Errors are reckoned in the decoder's own units: eighths of an 8-bit
    // step for R11.
    let scale = if kind == Eac::R11 {
        2047.0 / 255.0
    } else {
        1.0
    };
    let targets = values.map(|v| (v as f32 * scale).round() as i32);
    let decode = |base: i32, multiplier: i32, modifier: i32| match kind {
        Eac::Alpha8 => (base + modifier * multiplier).clamp(0, 255),
        Eac::R11 => (base * 8 + 4 + modifier * multiplier * 8).clamp(0, 2047),
    };
    let (low, high) = values.iter().fold((255, 0), |(lo, hi), &v| {
        (lo.min(v as i32), hi.max(v as i32))
    });
    let mut best = (i32::MAX, 0u64);
    for (table, modifiers) in EAC_TABLES.iter().enumerate() {
        let (m_low, m_high) = (modifiers[3], modifiers[7]);
        let ideal = (high - low) as f32 / (m_high - m_low) as f32;
        let around = ideal.round() as i32;
        for multiplier in (around - 1).max(1)..=(around + 1).min(15) {
            let centre =
                (high + low) as f32 / 2.0 - (m_high + m_low) as f32 * multiplier as f32 / 2.0;
            let around = centre.round() as i32;
            for base in (around - 1).max(0)..=(around + 1).min(255) {
                let mut error = 0;
                let mut indices = 0u64;
                for y in 0..4 {
                    for x in 0..4 {
                        let target = targets[y * 4 + x];
                        let (mut e_best, mut i_best) = (i32::MAX, 0);
                        for (index, &m) in modifiers.iter().enumerate() {
                            let e = square(decode(base, multiplier, m) - target);
                            if e < e_best {
                                e_best = e;
                                i_best = index as u64;
                            }
                        }
                        error += e_best;
                        indices |= i_best << (45 - 3 * slot(x, y));
                    }
                }
                if error < best.0 {
                    let word = (base as u64) << 56
                        | (multiplier as u64) << 52
                        | (table as u64) << 48
                        | indices;
                    best = (error, word);
                }
            }
        }
    }
    best.1.to_be_bytes()
}

/// The 16 texels of the block at block column `bx` and row `by`, each
/// `channels` bytes, the image's edge repeated past its last row and column.
fn gather<const N: usize>(
    data: &[u8],
    width: usize,
    height: usize,
    bx: usize,
    by: usize,
) -> [[u8; N]; 16] {
    std::array::from_fn(|i| {
        let x = (bx * 4 + i % 4).min(width - 1);
        let y = (by * 4 + i / 4).min(height - 1);
        let at = (y * width + x) * N;
        std::array::from_fn(|c| data[at + c])
    })
}

/// Compress an image block by block, `bytes` a block, rows of blocks spread
/// over the machine's cores.
fn compress<const N: usize>(
    data: &[u8],
    width: usize,
    height: usize,
    bytes: usize,
    encode: impl Fn(&[[u8; N]; 16]) -> Vec<u8> + Sync,
) -> Vec<u8> {
    assert_eq!(data.len(), width * height * N);
    let (columns, rows) = (width.div_ceil(4), height.div_ceil(4));
    let mut out = vec![0u8; columns * rows * bytes];
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let rows_each = rows.div_ceil(threads);
    std::thread::scope(|scope| {
        for (chunk, part) in out.chunks_mut(rows_each * columns * bytes).enumerate() {
            let encode = &encode;
            scope.spawn(move || {
                for (i, block) in part.chunks_mut(bytes).enumerate() {
                    let index = chunk * rows_each * columns + i;
                    let texels = gather::<N>(data, width, height, index % columns, index / columns);
                    block.copy_from_slice(&encode(&texels));
                }
            });
        }
    });
    out
}

/// RGBA8 texels as ETC2 RGBA8: EAC alpha, then ETC2 colour, each block.
pub fn compress_rgba(data: &[u8], width: usize, height: usize) -> Vec<u8> {
    compress::<4>(data, width, height, 16, |texels| {
        let alpha = eac_block(&texels.map(|t| t[3]), Eac::Alpha8);
        let colour = rgb_block(texels);
        [alpha, colour].concat()
    })
}

/// RGBA8 texels as ETC2 RGB8, alpha dropped.
pub fn compress_rgb(data: &[u8], width: usize, height: usize) -> Vec<u8> {
    compress::<4>(data, width, height, 8, |texels| rgb_block(texels).to_vec())
}

/// One channel as EAC R11.
pub fn compress_r11(data: &[u8], width: usize, height: usize) -> Vec<u8> {
    compress::<1>(data, width, height, 8, |texels| {
        eac_block(&texels.map(|t| t[0]), Eac::R11).to_vec()
    })
}

/// Two interleaved channels as EAC RG11.
pub fn compress_rg11(data: &[u8], width: usize, height: usize) -> Vec<u8> {
    compress::<2>(data, width, height, 16, |texels| {
        let red = eac_block(&texels.map(|t| t[0]), Eac::R11);
        let green = eac_block(&texels.map(|t| t[1]), Eac::R11);
        [red, green].concat()
    })
}

/// Bytes of one compressed level of `width` × `height`, `bytes` a block.
pub fn level_size(width: usize, height: usize, bytes: usize) -> usize {
    width.div_ceil(4) * height.div_ceil(4) * bytes
}

/// Decode one ETC2 RGB block written by [`rgb_block`] (the ETC1 modes and
/// planar; not T or H) to 16 RGB texels, row-major.
pub fn decode_rgb_block(block: &[u8]) -> [[u8; 3]; 16] {
    let word = u64::from_be_bytes(block[..8].try_into().expect("an 8-byte block"));
    let field = |at: u32, bits: u32| (word >> at & ((1 << bits) - 1)) as i32;
    let differential = field(33, 1) == 1;
    let delta = |at: u32| (field(at, 3) ^ 4) - 4;
    if differential && !(0..=31).contains(&(field(43, 5) + delta(40))) {
        let o = [
            expand(field(57, 6), 6),
            expand(field(56, 1) << 6 | field(49, 6), 7),
            expand(field(48, 1) << 5 | field(43, 2) << 3 | field(39, 3), 6),
        ];
        let h = [
            expand(field(34, 5) << 1 | field(32, 1), 6),
            expand(field(25, 7), 7),
            expand(field(19, 6), 6),
        ];
        let v = [
            expand(field(13, 6), 6),
            expand(field(6, 7), 7),
            expand(field(0, 6), 6),
        ];
        return std::array::from_fn(|i| {
            let (x, y) = ((i % 4) as i32, (i / 4) as i32);
            std::array::from_fn(|c| {
                ((x * (h[c] - o[c]) + y * (v[c] - o[c]) + 4 * o[c] + 2) >> 2).clamp(0, 255) as u8
            })
        });
    }
    let bases: [[i32; 3]; 2] = if differential {
        let first = [0, 1, 2].map(|c| field(59 - 8 * c as u32, 5));
        [
            first.map(|v| expand(v, 5)),
            [0, 1, 2].map(|c| expand(first[c] + delta(56 - 8 * c as u32), 5)),
        ]
    } else {
        [0, 1].map(|half| [0, 1, 2].map(|c| expand(field(60 - 4 * half - 8 * c as u32, 4), 4)))
    };
    let tables = [field(37, 3), field(34, 3)];
    let flip = field(32, 1) == 1;
    std::array::from_fn(|i| {
        let (x, y) = (i % 4, i / 4);
        let half = if flip { y / 2 } else { x / 2 };
        let s = slot(x, y) as u32;
        let index = field(16 + s, 1) << 1 | field(s, 1);
        let steps = ETC1_TABLES[tables[half] as usize];
        let m = [steps[0], steps[1], -steps[0], -steps[1]][index as usize];
        bases[half].map(|b| (b + m).clamp(0, 255) as u8)
    })
}

/// Decode one EAC block to 16 values, row-major, in 8 bits (R11 rounded).
pub fn decode_eac_block(block: &[u8], kind: Eac) -> [u8; 16] {
    let word = u64::from_be_bytes(block[..8].try_into().expect("an 8-byte block"));
    let base = (word >> 56) as i32;
    let multiplier = (word >> 52 & 15) as i32;
    let modifiers = EAC_TABLES[(word >> 48 & 15) as usize];
    std::array::from_fn(|i| {
        let index = (word >> (45 - 3 * slot(i % 4, i / 4)) & 7) as usize;
        let m = modifiers[index];
        match kind {
            Eac::Alpha8 => (base + m * multiplier).clamp(0, 255) as u8,
            Eac::R11 => {
                let v = if multiplier == 0 {
                    base * 8 + 4 + m
                } else {
                    base * 8 + 4 + m * multiplier * 8
                };
                (v.clamp(0, 2047) as f32 * 255.0 / 2047.0).round() as u8
            }
        }
    })
}

/// Decode a compressed image made by one of the `compress_*` functions back
/// to `channels` bytes a texel, for GPUs that cannot sample it compressed:
/// 4 for [`Layout::Rgba`] and [`Layout::Rgb`] (alpha 255), 1 for R11, 2 for RG11.
pub fn decompress(data: &[u8], width: usize, height: usize, layout: Layout) -> Vec<u8> {
    let channels = layout.channels();
    let bytes = layout.block_bytes();
    let columns = width.div_ceil(4);
    let mut out = vec![0u8; width * height * channels];
    for (index, block) in data.chunks_exact(bytes).enumerate() {
        let (bx, by) = (index % columns, index / columns);
        let texels: [[u8; 4]; 16] = match layout {
            Layout::Rgba => {
                let alpha = decode_eac_block(&block[..8], Eac::Alpha8);
                let colour = decode_rgb_block(&block[8..]);
                std::array::from_fn(|i| [colour[i][0], colour[i][1], colour[i][2], alpha[i]])
            }
            Layout::Rgb => decode_rgb_block(block).map(|c| [c[0], c[1], c[2], 255]),
            Layout::R11 => decode_eac_block(block, Eac::R11).map(|v| [v, 0, 0, 0]),
            Layout::Rg11 => {
                let red = decode_eac_block(&block[..8], Eac::R11);
                let green = decode_eac_block(&block[8..], Eac::R11);
                std::array::from_fn(|i| [red[i], green[i], 0, 0])
            }
        };
        for (i, texel) in texels.iter().enumerate() {
            let (x, y) = (bx * 4 + i % 4, by * 4 + i / 4);
            if x < width && y < height {
                let at = (y * width + x) * channels;
                out[at..at + channels].copy_from_slice(&texel[..channels]);
            }
        }
    }
    out
}

/// The block formats, by what they hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// ETC2 RGBA8: an EAC alpha block, then an ETC2 colour block.
    Rgba,
    /// ETC2 RGB8.
    Rgb,
    /// EAC R11.
    R11,
    /// EAC RG11: the red block, then the green.
    Rg11,
}

impl Layout {
    /// Bytes a 4 × 4 block.
    pub fn block_bytes(self) -> usize {
        match self {
            Layout::Rgba | Layout::Rg11 => 16,
            Layout::Rgb | Layout::R11 => 8,
        }
    }

    /// Bytes a texel, uncompressed.
    pub fn channels(self) -> usize {
        match self {
            Layout::Rgba | Layout::Rgb => 4,
            Layout::R11 => 1,
            Layout::Rg11 => 2,
        }
    }

    /// Compress `data`, [`Layout::channels`] bytes a texel.
    pub fn compress(self, data: &[u8], width: usize, height: usize) -> Vec<u8> {
        match self {
            Layout::Rgba => compress_rgba(data, width, height),
            Layout::Rgb => compress_rgb(data, width, height),
            Layout::R11 => compress_r11(data, width, height),
            Layout::Rg11 => compress_rg11(data, width, height),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Peak signal to noise ratio between two images over some channels, in dB.
    fn psnr(a: &[u8], b: &[u8], channels: usize, which: std::ops::Range<usize>) -> f64 {
        let mut sum = 0.0;
        let mut n = 0.0;
        for (x, y) in a.chunks(channels).zip(b.chunks(channels)) {
            for c in which.clone() {
                sum += (x[c] as f64 - y[c] as f64).powi(2);
                n += 1.0;
            }
        }
        if sum == 0.0 {
            return f64::INFINITY;
        }
        10.0 * (255.0f64.powi(2) / (sum / n)).log10()
    }

    /// A test card, RGBA: smooth ramps, grain, a hard curved edge and a mottle,
    /// with sides that are not multiples of four.
    fn card(width: usize, height: usize) -> Vec<u8> {
        let mut seed = 12345u32;
        let mut noise = move || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 24) as i32 - 128
        };
        (0..width * height)
            .flat_map(|i| {
                let (x, y) = (i % width, i / width);
                let ramp = |a: usize, b: usize| ((a * 255 / width + b * 255 / height) / 2) as i32;
                let c: [i32; 4] = match x * 4 / width {
                    0 => [ramp(x, 0), ramp(0, y), ramp(x, y), ramp(width - x, y)],
                    1 => {
                        let g = noise() / 5;
                        [120 + g, 90 + g, 60 + noise() / 8, 200 + noise() / 6]
                    }
                    2 => {
                        let (dx, dy) = (x as f32 - width as f32 * 0.6, y as f32 - height as f32);
                        if dx.hypot(dy) < height as f32 * 0.8 {
                            [250, 30, 40, 255]
                        } else {
                            [10, 60, 200, 0]
                        }
                    }
                    _ => [
                        20 + ramp(x, y) / 8,
                        40,
                        90 + noise() / 16,
                        128 + noise() / 4,
                    ],
                };
                c.map(|v| v.clamp(0, 255) as u8)
            })
            .collect()
    }

    #[test]
    fn rgba_round_trips() {
        let (width, height) = (62, 37);
        let texels = card(width, height);
        let data = compress_rgba(&texels, width, height);
        assert_eq!(data.len(), level_size(width, height, 16));
        let back = decompress(&data, width, height, Layout::Rgba);
        // Grain (white noise) in a quarter of the card keeps this low.
        let colour = psnr(&texels, &back, 4, 0..3);
        let alpha = psnr(&texels, &back, 4, 3..4);
        assert!(colour > 25.0, "colour PSNR {colour:.1} dB");
        assert!(alpha > 32.0, "alpha PSNR {alpha:.1} dB");
        // The smooth quarter must be near perfect.
        let quarter = |v: &[u8]| -> Vec<u8> {
            v.chunks(4)
                .enumerate()
                .filter(|(i, _)| (i % width) * 4 / width == 0)
                .flat_map(|(_, t)| t.to_vec())
                .collect()
        };
        let ramp = psnr(&quarter(&texels), &quarter(&back), 4, 0..3);
        assert!(ramp > 40.0, "ramp PSNR {ramp:.1} dB");
    }

    #[test]
    fn the_colour_decoder_agrees_with_an_independent_one() {
        // texture2ddecoder (its ETC2 colour; its EAC reads the index bits in
        // the wrong byte order, so the EAC decoder follows Mesa's instead).
        let (width, height) = (64, 36);
        let texels = card(width, height);
        let data = compress_rgb(&texels, width, height);
        let mine = decompress(&data, width, height, Layout::Rgb);
        let mut image = vec![0u32; width * height];
        texture2ddecoder::decode_etc2_rgb(&data, width, height, &mut image).unwrap();
        for (i, texel) in image.iter().enumerate() {
            let [b, g, r, _] = texel.to_le_bytes();
            assert_eq!([r, g, b], mine[i * 4..i * 4 + 3], "texel {i}");
        }
    }

    #[test]
    fn a_gradient_block_takes_planar_mode() {
        let texels: [[u8; 4]; 16] = std::array::from_fn(|i| {
            let (x, y) = ((i % 4) as u8, (i / 4) as u8);
            [10 + 20 * x, 200 - 15 * y, 60 + 7 * x + 9 * y, 255]
        });
        let block = rgb_block(&texels);
        // Blue out of range in the differential reading marks planar mode.
        let word = u64::from_be_bytes(block);
        let b = (word >> 43 & 31) as i32 + ((word >> 40 & 7) as i32 ^ 4) - 4;
        assert!(word >> 33 & 1 == 1 && !(0..=31).contains(&b));
        let back = decode_rgb_block(&block);
        for (a, b) in texels.iter().zip(&back) {
            for c in 0..3 {
                assert!((a[c] as i32 - b[c] as i32).abs() <= 3, "{a:?} {b:?}");
            }
        }
    }

    #[test]
    fn eac_channels_round_trip() {
        let (width, height) = (41, 23);
        let texels = card(width, height);
        let red: Vec<u8> = texels.chunks(4).map(|t| t[0]).collect();
        let back = decompress(
            &compress_r11(&red, width, height),
            width,
            height,
            Layout::R11,
        );
        let r = psnr(&red, &back, 1, 0..1);
        assert!(r > 36.0, "R11 PSNR {r:.1} dB");

        let pairs: Vec<u8> = texels.chunks(4).flat_map(|t| [t[1], t[2]]).collect();
        let back = decompress(
            &compress_rg11(&pairs, width, height),
            width,
            height,
            Layout::Rg11,
        );
        let rg = psnr(&pairs, &back, 2, 0..2);
        assert!(rg > 36.0, "RG11 PSNR {rg:.1} dB");
    }
}
