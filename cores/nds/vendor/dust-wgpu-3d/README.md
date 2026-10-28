# FANPLAY Dust wgpu-3d fork

This is Dust `76d79f25e4ea3ce355c4ac868f5ac9a233e64b89`'s `render/wgpu-3d`
crate. It is vendored so FANPLAY can restore the threaded GPU render submission
that is commented out in upstream, allowing the existing Dust 3D renderer to
draw frames for the paired wgpu-2d compositor.

Upstream: https://github.com/kelpsyberry/dust/tree/76d79f25e4ea3ce355c4ac868f5ac9a233e64b89/render/wgpu-3d
License: GPL-3.0-or-later (see `LICENSE`).
