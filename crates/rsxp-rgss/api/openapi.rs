// SPDX-License-Identifier: GPL-2.0-or-later
#![allow(dead_code)]
//! Code-first OpenAPI contract for the host-owned portion of the loopback API.
//!
//! Game and input routes belong to Rhai extensions. They are registered at
//! runtime and exposed through `/v1/extensions/routes`; this document only
//! describes that generic discovery endpoint.
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "rsxp Live Game API",
        version = "0.3.0",
        description = "Loopback API whose game and input routes are supplied by loaded Rhai plugins. Discover the active route catalog through /v1/extensions/routes."
    ),
    servers((url = "http://127.0.0.1:47821", description = "Running local game process")),
    tags((name = "Extensions")),
    paths(extension_routes),
    components(schemas(ExtensionRouteCatalog, ExtensionRoute))
)]
pub struct ApiDoc;

#[utoipa::path(
    get,
    path = "/v1/extensions/routes",
    tag = "Extensions",
    operation_id = "listExtensionRoutes",
    responses(
        (status = 200, description = "Routes exposed by the currently loaded Rhai plugins", body = ExtensionRouteCatalog)
    )
)]
fn extension_routes() {}

#[derive(utoipa::ToSchema)]
struct ExtensionRouteCatalog {
    routes: Vec<ExtensionRoute>,
}

#[derive(utoipa::ToSchema)]
struct ExtensionRoute {
    method: String,
    path: String,
    operation: String,
    arguments: Vec<String>,
}
