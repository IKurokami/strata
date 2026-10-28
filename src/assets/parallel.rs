// SPDX-License-Identifier: MIT

use super::*;
use futures_channel::oneshot;

struct Prepared {
    key: IconTextureKey,
    source: String,
    tint: Option<[f32; 4]>,
}

type Reply = oneshot::Sender<Option<gdk::Texture>>;
#[derive(Default)]
struct Queue {
    running: usize,
    jobs: VecDeque<Prepared>,
    waiting: HashMap<IconTextureKey, Vec<Reply>>,
}

thread_local! { static QUEUE: RefCell<Queue> = RefCell::default(); }

fn key(name: &str, color: &str, size: i32, scale: i32, context: IconContext) -> IconTextureKey {
    let palette = if name.starts_with(MATERIAL_ICON_PREFIX) {
        if CATPPUCCIN_THEME_COLORS.get() {
            format!("accent:{color}")
        } else {
            format!("material:{}", CATPPUCCIN_FLAVOR.get().name())
        }
    } else if name.starts_with(CATPPUCCIN_ICON_PREFIX) {
        if CATPPUCCIN_THEME_COLORS.get() {
            format!("accent:{color}")
        } else {
            CATPPUCCIN_FLAVOR.get().name().to_owned()
        }
    } else {
        color.to_owned()
    };
    (
        if name.starts_with(MATERIAL_ICON_PREFIX) {
            name.to_owned()
        } else {
            stroke_cache_name(name, size, context)
        },
        palette,
        size.saturating_mul(scale.max(1)),
    )
}

pub(crate) fn cached_file_icon_texture(
    name: &str,
    color: &str,
    size: i32,
    scale: i32,
    context: IconContext,
) -> Option<gdk::Texture> {
    cached_icon_texture(&key(name, color, size, scale, context))
}

pub(crate) async fn file_icon_texture(
    name: String,
    color: String,
    size: i32,
    scale: i32,
    context: IconContext,
    visible: bool,
) -> Option<gdk::Texture> {
    if let Some(texture) = cached_file_icon_texture(&name, &color, size, scale, context) {
        return Some(texture);
    }
    let key = key(&name, &color, size, scale, context);
    let (tx, rx) = oneshot::channel();
    let mut reply = Some(tx);
    let joined = QUEUE.with_borrow_mut(|queue| {
        if let Some(waiters) = queue.waiting.get_mut(&key) {
            waiters.push(reply.take().expect("new icon request"));
            if visible
                && let Some(index) = queue.jobs.iter().position(|job| job.key == key)
                && let Some(job) = queue.jobs.remove(index)
            {
                queue.jobs.push_back(job);
            }
            true
        } else {
            false
        }
    });
    if !joined {
        let prepared = prepare(&name, &color, key.2, size, context)?;
        QUEUE.with_borrow_mut(|queue| {
            while queue.jobs.len() >= 128 {
                if let Some(oldest) = queue.jobs.pop_front() {
                    queue.waiting.remove(&oldest.key);
                }
            }
            queue
                .waiting
                .insert(key, vec![reply.take().expect("unjoined icon request")]);
            if visible {
                queue.jobs.push_back(prepared);
            } else {
                queue.jobs.push_front(prepared);
            }
        });
    }
    start();
    rx.await.ok().flatten()
}

fn start() {
    loop {
        let next = QUEUE.with_borrow_mut(|queue| {
            if queue.running >= 4 {
                return None;
            }
            while let Some(job) = queue.jobs.pop_back() {
                let wanted = queue.waiting.get_mut(&job.key).is_some_and(|waiting| {
                    waiting.retain(|reply| !reply.is_canceled());
                    !waiting.is_empty()
                });
                if !wanted {
                    queue.waiting.remove(&job.key);
                    continue;
                }
                queue.running += 1;
                return Some(job);
            }
            None
        });
        let Some(job) = next else { return };
        glib::MainContext::default().spawn_local(async move {
            let pixels = job.key.2;
            let data = gio::spawn_blocking(move || {
                let mut data = super::vector::render_argb32(&job.source, pixels)?;
                if let Some([red, green, blue, alpha]) = job.tint {
                    for pixel in data.as_chunks_mut::<4>().0 {
                        let argb = u32::from_ne_bytes(*pixel);
                        let opacity = (argb >> 24) as f32 * alpha;
                        let argb = ((opacity as u32) << 24)
                            | (((red * opacity) as u32) << 16)
                            | (((green * opacity) as u32) << 8)
                            | (blue * opacity) as u32;
                        pixel.copy_from_slice(&argb.to_ne_bytes());
                    }
                }
                Some(data)
            })
            .await
            .ok()
            .flatten();
            let texture = data
                .and_then(|data| {
                    cairo::ImageSurface::create_for_data(
                        data,
                        cairo::Format::ARgb32,
                        pixels,
                        pixels,
                        pixels * 4,
                    )
                    .ok()
                })
                .and_then(|surface| texture_from_surface(&surface))
                .map(|texture| cache_icon_texture(job.key.clone(), texture));
            let waiting = QUEUE.with_borrow_mut(|queue| {
                queue.running = queue.running.saturating_sub(1);
                queue.waiting.remove(&job.key).unwrap_or_default()
            });
            for reply in waiting {
                let _ = reply.send(texture.clone());
            }
            start();
        });
    }
}

fn prepare(
    name: &str,
    color: &str,
    pixels: i32,
    logical: i32,
    context: IconContext,
) -> Option<Prepared> {
    let flavor = CATPPUCCIN_FLAVOR.get();
    let themed = CATPPUCCIN_THEME_COLORS.get();
    if name.starts_with(MATERIAL_ICON_PREFIX) {
        let palette = if themed {
            format!("accent:{color}")
        } else {
            format!("material:{}", flavor.name())
        };
        let resource = |name: &str| format!("/io/github/lgse/Strata/icons/material/{name}.svg");
        let data = (flavor == CatppuccinFlavor::Latte)
            .then(|| {
                gio::resources_lookup_data(
                    &resource(&format!("{name}_light")),
                    gio::ResourceLookupFlags::NONE,
                )
                .ok()
            })
            .flatten()
            .or_else(|| {
                gio::resources_lookup_data(&resource(name), gio::ResourceLookupFlags::NONE).ok()
            })?;
        let tint = if themed {
            let color = gdk::RGBA::parse(color).ok()?;
            Some([color.red(), color.green(), color.blue(), color.alpha()])
        } else {
            None
        };
        return Some(Prepared {
            key: (name.to_owned(), palette, pixels),
            source: std::str::from_utf8(data.as_ref()).ok()?.to_owned(),
            tint,
        });
    }
    let (directory, palette) = if name.starts_with(CATPPUCCIN_ICON_PREFIX) {
        (
            "catppuccin",
            if themed {
                format!("accent:{color}")
            } else {
                flavor.name().to_owned()
            },
        )
    } else {
        ("scalable/actions", color.to_owned())
    };
    let data = gio::resources_lookup_data(
        &format!("/io/github/lgse/Strata/icons/{directory}/{name}.svg"),
        gio::ResourceLookupFlags::NONE,
    )
    .ok()?;
    let mut source = std::str::from_utf8(data.as_ref()).ok()?.to_owned();
    if name.starts_with(CATPPUCCIN_ICON_PREFIX) {
        for (variable, hex) in flavor.palette() {
            source = source.replace(
                &format!("var(--vscode-ctp-{variable})"),
                if themed { color } else { hex },
            );
        }
        if themed {
            source = source.replace(
                "fill=\"none\"",
                &format!("fill=\"{color}\" fill-opacity=\"0.15\""),
            );
        }
        source = scale_catppuccin_strokes(
            source,
            icon_stroke_factor(logical, context) * catppuccin_stroke_weight(context),
        );
    } else {
        source = recolor_icon_source(&source, color);
        if name == icons::FOLDER {
            source = source.replacen(
                "fill=\"none\"",
                &format!("fill=\"{color}\" fill-opacity=\"0.15\""),
                1,
            );
        }
        source = svg_at_texture_size(compensate_icon_strokes(source, logical, context), pixels);
    }
    Some(Prepared {
        key: (stroke_cache_name(name, logical, context), palette, pixels),
        source,
        tint: None,
    })
}
