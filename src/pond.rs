//! A pond of koi: one per public, non-fork repository the account owns, each sized by
//! the commits on its default branch relative to the busiest repository, cruising among
//! a few lily pads and lotus flowers. Each koi wears its repository's language on the
//! caller's chosen body, with the same palette and shading as the single koi.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::f64::consts::{PI, TAU};

use chrono::NaiveDate;
use serde_json::Value;

use crate::behavior::Behavior;
use crate::coat::KoiBody;
use crate::collect::{
    CollectionError, GraphqlClient, read_array, read_bool, read_field, read_str, read_u64,
};
use crate::history::LanguageColor;
use crate::koi::{ADULT_CONTRIBUTIONS, KoiDevelopment, RecentActivity, StreakDevelopment};
use crate::motion::{FRAMES_PER_SECOND, frame_count};
use crate::render::{FRAME_HEIGHT_PIXELS, FRAME_WIDTH_PIXELS, render_swim_frame};

/// One repository's koi.
#[derive(Clone, Debug, PartialEq)]
pub struct PondKoi {
    /// GitHub's numeric repository ID; it seeds the koi's pattern and its place in the loop.
    pub repository_id: u64,
    /// The primary language and GitHub's color for it, if the repository has one.
    pub language: Option<(String, Option<LanguageColor>)>,
    /// Commits on the default branch; an empty repository has none.
    pub commit_count: u64,
}

/// The pond holds at most this many koi, the busiest repositories first.
// ponytail: fixed cap keeps the pond calm; make it a flag if someone wants a crowded pond.
pub const MAX_POND_KOI: usize = 12;

const POND_QUERY: &str = "query($login: String!, $cursor: String) { user(login: $login) { \
    repositories(first: 100, after: $cursor, ownerAffiliations: [OWNER], privacy: PUBLIC, \
    isFork: false) { nodes { databaseId primaryLanguage { name color } \
    defaultBranchRef { target { ... on Commit { history { totalCount } } } } } \
    pageInfo { hasNextPage endCursor } } } }";

/// Collect the account's public, non-fork repositories with their default-branch commit
/// counts, busiest first, keeping at most [`MAX_POND_KOI`].
pub fn collect_pond(
    github: &impl GraphqlClient,
    login: &str,
) -> Result<Vec<PondKoi>, CollectionError> {
    let mut pond = Vec::new();
    let mut page_cursor: Option<String> = None;
    loop {
        let mut variables = vec![("login", login)];
        if let Some(page_cursor) = &page_cursor {
            variables.push(("cursor", page_cursor.as_str()));
        }
        let page_data = github.run_query(POND_QUERY, &variables)?;
        let user = read_field(&page_data, "user")?;
        if user.is_null() {
            return Err(CollectionError::AccountNotFound(login.to_owned()));
        }
        let repositories = read_field(user, "repositories")?;
        for repository_node in read_array(repositories, "nodes")? {
            let language = match read_field(repository_node, "primaryLanguage")? {
                Value::Null => None,
                language => {
                    let language_color = match read_field(language, "color")? {
                        Value::Null => None,
                        color => Some(
                            LanguageColor::try_from(color.as_str().unwrap_or_default())
                                .map_err(CollectionError::UnexpectedResponse)?,
                        ),
                    };
                    Some((read_str(language, "name")?.to_owned(), language_color))
                }
            };
            pond.push(PondKoi {
                repository_id: read_u64(repository_node, "databaseId")?,
                language,
                // An empty repository has no default branch.
                commit_count: repository_node
                    .pointer("/defaultBranchRef/target/history/totalCount")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            });
        }
        let page_info = read_field(repositories, "pageInfo")?;
        if !read_bool(page_info, "hasNextPage")? {
            break;
        }
        let next_cursor = read_str(page_info, "endCursor")?.to_owned();
        if page_cursor.as_deref() == Some(next_cursor.as_str()) {
            return Err(CollectionError::UnexpectedResponse(format!(
                "repositories repeated page cursor {next_cursor}"
            )));
        }
        page_cursor = Some(next_cursor);
    }
    pond.sort_by(|left, right| {
        right
            .commit_count
            .cmp(&left.commit_count)
            .then(left.repository_id.cmp(&right.repository_id))
    });
    pond.truncate(MAX_POND_KOI);
    Ok(pond)
}

const POND_BEHAVIOR: Behavior = Behavior::Cruise;
/// Distance between neighboring koi's lap centers; laps overlap a little, like a real pond.
const CELL_PIXELS: u32 = 160;
const MARGIN_PIXELS: u32 = 32;
const CORNER_RADIUS_PIXELS: f64 = 18.0;
const RIM_PIXELS: f64 = 2.0;

const WATER: [u8; 4] = [18, 78, 137, 255];
const RIPPLE: [u8; 4] = [0, 153, 219, 255];
const RIM: [u8; 4] = [38, 43, 68, 255];
const OUTLINE: [u8; 4] = [24, 20, 37, 255];
const PAD_DARK: [u8; 4] = [38, 92, 66, 255];
const PAD: [u8; 4] = [62, 137, 72, 255];
const PAD_LIGHT: [u8; 4] = [99, 199, 77, 255];
const PETAL: [u8; 4] = [246, 117, 122, 255];
const PETAL_TIP: [u8; 4] = [181, 80, 136, 255];
const STAMEN: [u8; 4] = [254, 231, 97, 255];

/// Lily pads as fractions of the pond's size, a radius, and whether a lotus blooms on it.
// ponytail: fixed spots near the edges keep the koi in view; seed them if ponds start to look alike.
const LILY_PADS: [([f64; 2], f64, bool); 5] = [
    ([0.07, 0.16], 18.0, true),
    ([0.92, 0.84], 16.0, true),
    ([0.9, 0.1], 11.0, false),
    ([0.1, 0.88], 12.0, false),
    ([0.55, 0.06], 10.0, false),
];

/// Render the pond as a looping, opaque-inside GIF at native size. The koi cruise the
/// same loop as the single koi, each starting at its own point in the loop, and
/// neighbors circle in opposite directions. Bigger koi swim beneath smaller ones, and the
/// lily pads float above them all. Identical inputs produce identical bytes.
pub fn render_pond_gif(pond: &[PondKoi], koi_body: KoiBody) -> Result<Vec<u8>, String> {
    let koi_count = pond.len().max(1) as u32;
    // Roughly three columns for every two rows.
    let column_count = ((f64::from(koi_count) * 1.5).sqrt().ceil() as u32).clamp(1, koi_count);
    let row_count = koi_count.div_ceil(column_count);
    let width = column_count * CELL_PIXELS + 2 * MARGIN_PIXELS;
    let height = row_count * CELL_PIXELS + 2 * MARGIN_PIXELS;
    let busiest = pond
        .iter()
        .map(|koi| koi.commit_count)
        .max()
        .unwrap_or(0)
        .max(1);
    // Slots go by a hash of each repository, so sizes mix instead of running big to small.
    let mut slot_order: Vec<usize> = (0..pond.len()).collect();
    slot_order.sort_by_key(|&koi_index| mix(pond[koi_index].repository_id));
    // Draw order is busiest first, so the biggest koi swim beneath the smaller ones.
    let placed_koi: Vec<(KoiDevelopment, [i64; 2], bool, u32)> = pond
        .iter()
        .enumerate()
        .map(|(koi_index, pond_koi)| {
            let slot = slot_order
                .iter()
                .position(|&placed| placed == koi_index)
                .unwrap_or(0) as u32;
            let (column, row) = (slot % column_count, slot / column_count);
            let koi_in_row = (koi_count - row * column_count).min(column_count);
            // Center a short last row.
            let row_offset = (column_count - koi_in_row) * CELL_PIXELS / 2;
            let center = [
                (MARGIN_PIXELS + row_offset + column * CELL_PIXELS + CELL_PIXELS / 2) as i64,
                (MARGIN_PIXELS + row * CELL_PIXELS + CELL_PIXELS / 2) as i64,
            ];
            let is_mirrored = (column + row) % 2 == 1;
            let start_frame =
                (mix(pond_koi.repository_id) % u64::from(frame_count(POND_BEHAVIOR))) as u32;
            (
                develop_koi(pond_koi, busiest),
                center,
                is_mirrored,
                start_frame,
            )
        })
        .collect();
    let background = paint_background(width, height);
    let foreground = paint_foreground(width, height);
    let mut gif_encoder = gif::Encoder::new(Vec::new(), width as u16, height as u16, &[])
        .map_err(|error| error.to_string())?;
    gif_encoder
        .set_repeat(gif::Repeat::Infinite)
        .map_err(|error| error.to_string())?;
    let mut previous_pixels: Option<Vec<[u8; 4]>> = None;
    for frame_index in 0..frame_count(POND_BEHAVIOR) {
        let mut pixels = background.clone();
        for (koi_development, center, is_mirrored, start_frame) in &placed_koi {
            let koi_frame = render_swim_frame(
                koi_development,
                koi_body,
                POND_BEHAVIOR,
                frame_index + start_frame,
            );
            let koi_pixels = koi_frame.as_rgba_bytes().as_chunks::<4>().0;
            for tile_y in 0..FRAME_HEIGHT_PIXELS {
                for tile_x in 0..FRAME_WIDTH_PIXELS {
                    let koi_pixel = koi_pixels[(tile_y * FRAME_WIDTH_PIXELS + tile_x) as usize];
                    if koi_pixel[3] == 0 {
                        continue;
                    }
                    let source_x = if *is_mirrored {
                        FRAME_WIDTH_PIXELS - 1 - tile_x
                    } else {
                        tile_x
                    };
                    let pond_x =
                        center[0] - i64::from(FRAME_WIDTH_PIXELS / 2) + i64::from(source_x);
                    let pond_y = center[1] - i64::from(FRAME_HEIGHT_PIXELS / 2) + i64::from(tile_y);
                    if (0..i64::from(width)).contains(&pond_x)
                        && (0..i64::from(height)).contains(&pond_y)
                    {
                        pixels[(pond_y as u32 * width + pond_x as u32) as usize] = koi_pixel;
                    }
                }
            }
        }
        for (pixel, foreground_pixel) in pixels.iter_mut().zip(&foreground) {
            if let Some(foreground_pixel) = foreground_pixel {
                *pixel = *foreground_pixel;
            }
        }
        // Index zero is transparent: outside the pond, or unchanged since the last frame.
        let mut palette_indices: BTreeMap<[u8; 3], u8> = BTreeMap::new();
        let mut palette_channels = vec![0; 3];
        let mut frame_indices = Vec::with_capacity(pixels.len());
        for (pixel_index, &[red, green, blue, alpha]) in pixels.iter().enumerate() {
            let is_unchanged = previous_pixels
                .as_ref()
                .is_some_and(|previous| previous[pixel_index] == pixels[pixel_index]);
            if alpha == 0 || is_unchanged {
                frame_indices.push(0);
                continue;
            }
            let palette_index = match palette_indices.get(&[red, green, blue]) {
                Some(&palette_index) => palette_index,
                None if palette_indices.len() == 255 => {
                    return Err(format!("pond frame {frame_index} has more than 255 colors"));
                }
                None => {
                    let palette_index = palette_indices.len() as u8 + 1;
                    palette_indices.insert([red, green, blue], palette_index);
                    palette_channels.extend([red, green, blue]);
                    palette_index
                }
            };
            frame_indices.push(palette_index);
        }
        let elapsed = |frame: u32| (frame * 200 + FRAMES_PER_SECOND) / (2 * FRAMES_PER_SECOND);
        gif_encoder
            .write_frame(&gif::Frame {
                delay: (elapsed(frame_index + 1) - elapsed(frame_index)) as u16,
                dispose: gif::DisposalMethod::Keep,
                transparent: Some(0),
                width: width as u16,
                height: height as u16,
                palette: Some(palette_channels),
                buffer: Cow::Owned(frame_indices),
                ..gif::Frame::default()
            })
            .map_err(|error| error.to_string())?;
        previous_pixels = Some(pixels);
    }
    gif_encoder.into_inner().map_err(|error| error.to_string())
}

/// The repository as a koi: its language on the body, its commits, scaled so the busiest
/// repository is an adult, as growth. Streaks belong to the account, so pond koi have none.
fn develop_koi(pond_koi: &PondKoi, busiest: u64) -> KoiDevelopment {
    let (language_shares, language_colors) = match &pond_koi.language {
        Some((name, color)) => (
            BTreeMap::from([(name.clone(), 1.0)]),
            BTreeMap::from([(name.clone(), *color)]),
        ),
        None => (BTreeMap::new(), BTreeMap::new()),
    };
    KoiDevelopment {
        identity_seed: pond_koi.repository_id,
        language_shares,
        language_colors,
        total_contributions: pond_koi.commit_count * ADULT_CONTRIBUTIONS / busiest,
        observed_on: NaiveDate::default(),
        streaks: StreakDevelopment {
            longest: Vec::new(),
            recent: None,
        },
        recent_activity: RecentActivity {
            quiet_day_count: 0,
            recent_contributions: 0,
            complete_week_contributions: Vec::new(),
            observation_week_contributions: Vec::new(),
        },
    }
}

/// Water with sparse, still ripples.
fn paint_background(width: u32, height: u32) -> Vec<[u8; 4]> {
    (0..height)
        .flat_map(|pixel_y| (0..width).map(move |pixel_x| (pixel_x, pixel_y)))
        .map(|(pixel_x, pixel_y)| {
            let is_ripple = pixel_y % 11 == 5
                && pixel_x % 9 < 4
                && mix(u64::from(pixel_x / 9) << 32 | u64::from(pixel_y)).is_multiple_of(17);
            if is_ripple { RIPPLE } else { WATER }
        })
        .collect()
}

/// What floats above the koi: lily pads and lotus flowers with the koi's outline, the
/// pond's rim, and clear corners. `Some` replaces the pixel beneath; a clear pixel
/// makes it transparent.
fn paint_foreground(width: u32, height: u32) -> Vec<Option<[u8; 4]>> {
    let mut foreground = vec![None; (width * height) as usize];
    for (pad_index, &([fraction_x, fraction_y], radius, has_lotus)) in LILY_PADS.iter().enumerate()
    {
        let center = [
            fraction_x * f64::from(width),
            fraction_y * f64::from(height),
        ];
        let notch_angle = pad_index as f64 * 2.3 + 0.6;
        let lotus_center = [center[0] + radius * 0.2, center[1] - radius * 0.2];
        for pixel_y in 0..height {
            for pixel_x in 0..width {
                let offset = [
                    f64::from(pixel_x) + 0.5 - center[0],
                    f64::from(pixel_y) + 0.5 - center[1],
                ];
                let distance = offset[0].hypot(offset[1]);
                if distance > radius + 9.0 {
                    continue;
                }
                let pixel = &mut foreground[(pixel_y * width + pixel_x) as usize];
                if has_lotus
                    && let Some(lotus_color) = sample_lotus(
                        f64::from(pixel_x) + 0.5 - lotus_center[0],
                        f64::from(pixel_y) + 0.5 - lotus_center[1],
                    )
                {
                    *pixel = Some(lotus_color);
                    continue;
                }
                let angle_from_notch =
                    (offset[1].atan2(offset[0]) - notch_angle + PI).rem_euclid(TAU) - PI;
                if distance > radius || (angle_from_notch.abs() < 0.3 && distance > 1.5) {
                    continue;
                }
                *pixel = Some(if distance > radius - 1.5 {
                    PAD_DARK
                } else if distance > radius - 4.0 && offset[0] + offset[1] < -radius * 0.7 {
                    PAD_LIGHT
                } else {
                    PAD
                });
            }
        }
    }
    let shapes = foreground.clone();
    let width_index = width as usize;
    for (pixel_index, pixel) in foreground.iter_mut().enumerate() {
        if shapes[pixel_index].is_some() {
            continue;
        }
        let (pixel_x, pixel_y) = (pixel_index % width_index, pixel_index / width_index);
        let touches_shape = (pixel_x > 0 && shapes[pixel_index - 1].is_some())
            || (pixel_x + 1 < width_index && shapes[pixel_index + 1].is_some())
            || (pixel_y > 0 && shapes[pixel_index - width_index].is_some())
            || (pixel_index + width_index < shapes.len()
                && shapes[pixel_index + width_index].is_some());
        if touches_shape {
            *pixel = Some(OUTLINE);
        }
    }
    for pixel_y in 0..height {
        for pixel_x in 0..width {
            let inset = distance_inside_rounded_rect(pixel_x, pixel_y, width, height);
            let pixel = &mut foreground[(pixel_y * width + pixel_x) as usize];
            if inset < 0.0 {
                *pixel = Some([0; 4]);
            } else if inset < RIM_PIXELS {
                *pixel = Some(RIM);
            }
        }
    }
    foreground
}

/// A five-petal lotus seen from above, about thirteen pixels across; `None` outside it.
fn sample_lotus(offset_x: f64, offset_y: f64) -> Option<[u8; 4]> {
    const PETAL_REACH: f64 = 4.5;
    const PETAL_RADIUS: f64 = 3.3;
    let distance = offset_x.hypot(offset_y);
    if distance < 2.0 {
        return Some(STAMEN);
    }
    let in_petal = (0..5).any(|petal_index| {
        let angle = f64::from(petal_index) * TAU / 5.0 - PI / 2.0;
        (offset_x - PETAL_REACH * angle.cos()).hypot(offset_y - PETAL_REACH * angle.sin())
            < PETAL_RADIUS
    });
    in_petal.then_some(if distance > 6.4 { PETAL_TIP } else { PETAL })
}

/// How far a pixel's center lies inside the pond's rounded rectangle; negative outside.
fn distance_inside_rounded_rect(pixel_x: u32, pixel_y: u32, width: u32, height: u32) -> f64 {
    let half = [f64::from(width) / 2.0, f64::from(height) / 2.0];
    let offset = [
        (f64::from(pixel_x) + 0.5 - half[0]).abs() - (half[0] - CORNER_RADIUS_PIXELS),
        (f64::from(pixel_y) + 0.5 - half[1]).abs() - (half[1] - CORNER_RADIUS_PIXELS),
    ];
    let outside = offset[0].max(0.0).hypot(offset[1].max(0.0));
    CORNER_RADIUS_PIXELS - outside - offset[0].max(offset[1]).min(0.0)
}

/// SplitMix64's finalizer: a stable hash for placement and ripples.
fn mix(value: u64) -> u64 {
    let mut mixed = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    mixed ^ (mixed >> 31)
}
