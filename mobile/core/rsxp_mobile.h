// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once
#include <stdint.h>
#include <stddef.h>
#ifdef __cplusplus
extern "C" {
#endif
// Configure once before requests. All engine requests must use the same thread.
int rsxp_mobile_configure(const char *library_directory, const char *native_ruby_directory);
// Returns owned UTF-8 JSON: {"ok":value} or {"error":message}.
char *rsxp_mobile_request(const char *json);
void rsxp_mobile_free(char *response);
// Borrowed RGBA8 bytes, valid until the next frame call on this same thread.
// Copy/consume immediately; do not pass this pointer to another thread.
const unsigned char *rsxp_mobile_frame(uint32_t *width, uint32_t *height, size_t *length);
// Attach a CAMetalLayer (iOS) for zero-copy GPU presentation. Call before a game starts.
int rsxp_mobile_attach_surface(void *layer, uint32_t width, uint32_t height);
// Update the drawable size in physical pixels after rotation or layout changes.
int rsxp_mobile_resize_surface(uint32_t width, uint32_t height);
// 1 when the active game composites and presents through the GPU.
int rsxp_mobile_gpu_active(void);
#ifdef __cplusplus
}
#endif
