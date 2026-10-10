//! The MCP `resources` capability: every VM's screen, as text and as a PNG,
//! addressed by URI (`resources/list`, `resources/templates/list`,
//! `resources/read`). The content is what the `screen_text` and `screenshot`
//! tools return, reachable without a tool call — a client can attach
//! `cocovm://vm/<slug>/screen.txt` to a prompt the way it attaches a file.
//!
//! Subscriptions and `listChanged` notifications are not offered: both need
//! a server-to-client stream, and the transport answers with plain JSON
//! (`control::http` offers no SSE stream).

use serde::Deserialize;
use serde_json::{Value, json};

use super::jsonrpc::{INTERNAL_ERROR, INVALID_PARAMS, RpcError};
use super::protocol::{Action, ControlError, Reply, Request, VmInfo, VmStatus};
use super::tools::{Backend, UNEXPECTED_REPLY};

/// MCP's error code for a `resources/read` of a URI the server does not
/// serve (spec: server features → resources → error handling).
pub const RESOURCE_NOT_FOUND: i32 = -32002;

/// Every resource URI is `cocovm://vm/<slug>/<file>`.
const URI_PREFIX: &str = "cocovm://vm/";
/// The template variable `resources/templates/list` puts where a slug goes
/// (RFC 6570 level 1).
const SLUG_TEMPLATE_VARIABLE: &str = "{slug}";

/// What a VM exposes as a resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScreenResource {
    /// `screen.txt`: the decoded text screen, one line per row.
    Text,
    /// `screen.png`: the framebuffer, PNG-encoded.
    Png,
}

impl ScreenResource {
    const ALL: [ScreenResource; 2] = [ScreenResource::Text, ScreenResource::Png];

    /// The last path segment of the URI.
    fn file_name(self) -> &'static str {
        match self {
            ScreenResource::Text => "screen.txt",
            ScreenResource::Png => "screen.png",
        }
    }

    fn mime_type(self) -> &'static str {
        match self {
            ScreenResource::Text => "text/plain",
            ScreenResource::Png => "image/png",
        }
    }

    /// The resource's `name`, unique per VM: `<slug> screen text`.
    fn name(self, slug: &str) -> String {
        format!("{slug} {}", self.label())
    }

    /// The display `title` (protocol 2025-06-18 on): `<VM name> screen text`.
    fn title(self, vm_name: &str) -> String {
        format!("{vm_name} {}", self.label())
    }

    fn label(self) -> &'static str {
        match self {
            ScreenResource::Text => "screen text",
            ScreenResource::Png => "screenshot",
        }
    }

    fn describe(self, vm: &VmInfo) -> String {
        format!(
            "{} of VM '{}' ({}, {}).",
            self.summary(),
            vm.name,
            vm.slug,
            status_text(vm.status)
        )
    }

    /// The template description, which names no VM.
    fn describe_template(self) -> String {
        format!(
            "{} of the VM whose manager slug is {SLUG_TEMPLATE_VARIABLE}; list_vms reports the slugs.",
            self.summary()
        )
    }

    fn summary(self) -> &'static str {
        match self {
            ScreenResource::Text => {
                "The text screen decoded as lines (32x16, or 40/80 columns in CoCo 3 hi-res text \
                 modes); a graphics mode yields a one-line note instead"
            }
            ScreenResource::Png => {
                "The framebuffer as a PNG (GIME graphics modes are 640x240 with non-square pixels)"
            }
        }
    }

    fn action(self) -> Action {
        match self {
            ScreenResource::Text => Action::ScreenText,
            ScreenResource::Png => Action::Screenshot,
        }
    }

    fn uri(self, slug: &str) -> String {
        format!("{URI_PREFIX}{slug}/{}", self.file_name())
    }

    fn uri_template(self) -> String {
        self.uri(SLUG_TEMPLATE_VARIABLE)
    }

    /// `uri` split into the slug and the resource it names, if it has the
    /// shape [`Self::uri`] produces.
    fn parse_uri(uri: &str) -> Option<(&str, ScreenResource)> {
        let path = uri.strip_prefix(URI_PREFIX)?;
        let (slug, file_name) = path.split_once('/')?;
        if slug.is_empty() {
            return None;
        }
        let resource = Self::ALL.into_iter().find(|r| r.file_name() == file_name)?;
        Some((slug, resource))
    }
}

/// How a VM's state reads in a resource description: a powered-off or
/// suspended VM's resources exist but reading them fails until it runs.
fn status_text(status: VmStatus) -> &'static str {
    match status {
        VmStatus::Running => "running; readable now",
        VmStatus::Suspended => "suspended",
        VmStatus::PoweredOff => "powered off; start_vm makes it readable",
    }
}

/// The `resources` entry of the `initialize` capabilities.
pub fn capability() -> Value {
    json!({"subscribe": false, "listChanged": false})
}

#[derive(Deserialize)]
struct ListParams {
    #[serde(default)]
    cursor: Option<String>,
}

/// The `resources/list` result: two resources per VM the manager knows.
/// The whole list fits one page, so any `cursor` is one this server never
/// issued. `include_titles` adds each resource's display `title` (protocol
/// 2025-06-18 on).
pub fn list(
    backend: &mut dyn Backend,
    params: Value,
    include_titles: bool,
) -> Result<Value, RpcError> {
    let ListParams { cursor } = parse_params(params)?;
    if cursor.is_some() {
        return Err(RpcError::new(INVALID_PARAMS, "unknown cursor"));
    }
    let vms = list_vms(backend)?;
    let resources: Vec<Value> = vms
        .iter()
        .flat_map(|vm| {
            ScreenResource::ALL
                .into_iter()
                .map(move |resource| resource_json(resource, vm, include_titles))
        })
        .collect();
    Ok(json!({"resources": resources}))
}

fn resource_json(resource: ScreenResource, vm: &VmInfo, include_title: bool) -> Value {
    let mut value = json!({
        "uri": resource.uri(&vm.slug),
        "name": resource.name(&vm.slug),
        "description": resource.describe(vm),
        "mimeType": resource.mime_type(),
    });
    if include_title {
        value["title"] = json!(resource.title(&vm.name));
    }
    value
}

/// The `resources/templates/list` result: the two URI shapes, with the
/// slug as the template variable.
pub fn templates(include_titles: bool) -> Value {
    let templates: Vec<Value> = ScreenResource::ALL
        .into_iter()
        .map(|resource| {
            let mut value = json!({
                "uriTemplate": resource.uri_template(),
                "name": resource.name("VM"),
                "description": resource.describe_template(),
                "mimeType": resource.mime_type(),
            });
            if include_titles {
                value["title"] = json!(resource.title("VM"));
            }
            value
        })
        .collect();
    json!({"resourceTemplates": templates})
}

#[derive(Deserialize)]
struct ReadParams {
    uri: String,
}

/// Dispatch one `resources/read`: the VM's screen text as a `text`
/// content, or its framebuffer as a base64 `blob`. A URI this server does
/// not serve, including one naming a VM the manager doesn't know, is
/// [`RESOURCE_NOT_FOUND`]; a VM that exists but can't be read (not running)
/// is an [`INTERNAL_ERROR`] carrying the app's message.
pub fn read(backend: &mut dyn Backend, params: Value) -> Result<Value, RpcError> {
    let ReadParams { uri } = parse_params(params)?;
    let Some((slug, resource)) = ScreenResource::parse_uri(&uri) else {
        return Err(not_found(&uri));
    };
    let req = Request {
        vm: Some(slug.to_string()),
        action: resource.action(),
    };
    let reply = match backend.call(&req) {
        Ok(reply) => reply,
        Err(error) => return Err(read_error(backend, slug, &uri, error)),
    };
    let contents = match (resource, reply) {
        (ScreenResource::Text, Reply::Screen(screen)) => json!({
            "uri": uri,
            "mimeType": resource.mime_type(),
            "text": screen.lines.join("\n"),
        }),
        (ScreenResource::Png, Reply::Screenshot { png_base64, .. }) => json!({
            "uri": uri,
            "mimeType": resource.mime_type(),
            "blob": png_base64,
        }),
        _ => {
            return Err(RpcError::new(INTERNAL_ERROR, UNEXPECTED_REPLY));
        }
    };
    Ok(json!({"contents": [contents]}))
}

/// Classify a failed read: the manager answers an unknown slug and a VM
/// that isn't running with the same `Err`, so ask it which one this was.
/// A listing that fails itself reports the original error.
fn read_error(backend: &mut dyn Backend, slug: &str, uri: &str, error: ControlError) -> RpcError {
    match list_vms(backend) {
        Ok(vms) if !vms.iter().any(|vm| vm.slug == slug) => not_found(uri),
        _ => RpcError::new(INTERNAL_ERROR, error.message),
    }
}

fn list_vms(backend: &mut dyn Backend) -> Result<Vec<VmInfo>, RpcError> {
    let req = Request {
        vm: None,
        action: Action::ListVms,
    };
    match backend.call(&req) {
        Ok(Reply::Vms(vms)) => Ok(vms),
        Ok(_) => Err(RpcError::new(INTERNAL_ERROR, UNEXPECTED_REPLY)),
        Err(error) => Err(RpcError::new(INTERNAL_ERROR, error.message)),
    }
}

fn not_found(uri: &str) -> RpcError {
    RpcError::new(RESOURCE_NOT_FOUND, format!("no resource at '{uri}'"))
}

fn parse_params<T: serde::de::DeserializeOwned>(params: Value) -> Result<T, RpcError> {
    let params = match params {
        Value::Null => json!({}),
        other => other,
    };
    serde_json::from_value(params)
        .map_err(|e| RpcError::new(INVALID_PARAMS, format!("invalid params: {e}")))
}

#[cfg(test)]
#[path = "resources_test.rs"]
mod tests;
