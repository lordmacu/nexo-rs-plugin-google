//! Tool descriptions + JSON-Schema for the four `google_*` tools.
//!
//! `tool_defs()` produces the array shipped to the daemon via
//! `PluginAdapter::declare_tools` so the initialize-reply
//! advertises them; the daemon's `RemoteToolHandler` then
//! registers per-agent dispatch handlers using these defs.

use nexo_microapp_sdk::plugin::ToolDef;
use serde_json::json;

/// Full descriptor list. Order is preserved; daemon iterates this
/// array to build its tool registry.
pub fn tool_defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "google_auth_start".into(),
            description: "Begin the Google OAuth consent flow. Returns a URL the user must \
                open in a browser to approve access. The agent must forward the URL to the \
                user via chat and then stop calling Google tools until `google_auth_status` \
                reports authenticated. The callback listener binds on 127.0.0.1 inside the \
                plugin subprocess — remote hosts need an SSH tunnel \
                (`ssh -L <port>:127.0.0.1:<port> <host>`). Only call this when \
                `google_auth_status.authenticated` is false — re-calling it invalidates any \
                in-flight consent."
                .into(),
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        ToolDef {
            name: "google_auth_status".into(),
            description: "Report the current Google OAuth state: `authenticated` (bool), \
                `expires_in_secs` (access token TTL), `has_refresh` (can auto-renew), \
                `scopes` (what access was granted). Safe to call repeatedly — does not touch \
                the network; just reads the on-file tokens. When `authenticated` is false, \
                call `google_auth_start` to kick off the consent flow."
                .into(),
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        ToolDef {
            name: "google_call".into(),
            description: "Make an authenticated HTTP request against any `*.googleapis.com` \
                endpoint. Method is one of GET, POST, PUT, PATCH, DELETE. `body` (optional) \
                is a JSON value sent as the request payload. The access token is attached \
                as `Authorization: Bearer` and refreshed transparently when stale. Returns \
                the parsed JSON response.\n\nExamples:\n\
                - Gmail inbox: `{method: 'GET', url: 'https://gmail.googleapis.com/gmail/v1/users/me/messages?maxResults=10'}`\n\
                - Calendar insert: `{method: 'POST', url: 'https://www.googleapis.com/calendar/v3/calendars/primary/events', body: {...}}`\n\
                - Drive list: `{method: 'GET', url: 'https://www.googleapis.com/drive/v3/files?pageSize=50'}`\n\n\
                A 401 typically means the refresh_token was revoked — call \
                `google_auth_start` again. A 403 means the scope wasn't granted; update \
                `google_auth.scopes` in the agent config and re-auth."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "method": {
                        "type": "string",
                        "enum": ["GET", "POST", "PUT", "PATCH", "DELETE"],
                        "description": "HTTP method."
                    },
                    "url": {
                        "type": "string",
                        "description": "Full URL. Must be https:// — Google API hosts only."
                    },
                    "body": {
                        "type": "object",
                        "description": "Optional JSON payload (POST/PUT/PATCH)."
                    }
                },
                "required": ["method", "url"]
            }),
        },
        ToolDef {
            name: "google_auth_revoke".into(),
            description: "Revoke the agent's Google refresh_token (so even a leaked copy of \
                the file becomes useless) and delete the local tokens file. The user can \
                still see the agent at myaccount.google.com → Security → Third-party apps \
                until Google's cache clears (minutes). Next `google_call` will fail until \
                the user re-authorises via `google_auth_start`."
                .into(),
            input_schema: json!({ "type": "object", "properties": {} }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defines_four_tools_in_order() {
        let defs = tool_defs();
        assert_eq!(defs.len(), 4);
        assert_eq!(defs[0].name, "google_auth_start");
        assert_eq!(defs[1].name, "google_auth_status");
        assert_eq!(defs[2].name, "google_call");
        assert_eq!(defs[3].name, "google_auth_revoke");
    }

    #[test]
    fn google_call_input_schema_requires_method_and_url() {
        let defs = tool_defs();
        let call = defs.iter().find(|d| d.name == "google_call").unwrap();
        let required = call.input_schema["required"].as_array().unwrap();
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"method"));
        assert!(names.contains(&"url"));
    }
}
