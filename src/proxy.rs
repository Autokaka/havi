use crate::cef::resource::make_handler_with_headers;
use cef::*;
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[cfg_attr(feature = "napi-binding", napi(object))]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProxyRule {
    pub pattern: String,
    pub to: Option<String>,
    pub pass: Option<bool>,
    pub block: Option<bool>,
    pub status: Option<u32>,
    pub body: Option<String>,
    pub headers: Option<HashMap<String, String>>,
}

pub struct Compiled {
    rules: Vec<ProxyRule>,
    set: GlobSet,
}

impl Compiled {
    pub fn from_json(s: &str) -> Result<Self, String> {
        let rules: Vec<ProxyRule> = serde_json::from_str(s).map_err(|e| e.to_string())?;
        Self::from_rules(rules)
    }
    pub fn from_rules(rules: Vec<ProxyRule>) -> Result<Self, String> {
        let mut b = GlobSetBuilder::new();
        for r in &rules {
            b.add(Glob::new(&r.pattern).map_err(|e| e.to_string())?);
        }
        let set = b.build().map_err(|e| e.to_string())?;
        Ok(Self { rules, set })
    }
    pub fn first_match(&self, url: &str) -> Option<&ProxyRule> {
        self.set.matches(url).into_iter().next().map(|i| &self.rules[i])
    }
}

fn rewrite_url(pattern: &str, to: &str, url: &str) -> String {
    let glob_start = pattern
        .find(|c: char| matches!(c, '*' | '?' | '[' | '{'))
        .unwrap_or(pattern.len());
    let prefix = &pattern[..glob_start];
    if url.starts_with(prefix) {
        format!("{}{}", to, &url[prefix.len()..])
    } else {
        url.to_string()
    }
}

fn mime_of(h: &HashMap<String, String>) -> String {
    h.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| "text/plain".into())
}

wrap_resource_request_handler! {
    pub struct ProxyResReqHandler { pub compiled: Arc<Compiled> }
    impl ResourceRequestHandler {
        fn on_before_resource_load(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            request: Option<&mut Request>,
            _callback: Option<&mut Callback>,
        ) -> ReturnValue {
            let req = match request { Some(r) => r, None => return ReturnValue::CONTINUE };
            let url = CefString::from(&req.url()).to_string();
            let rule = match self.compiled.first_match(&url) { Some(r) => r, None => return ReturnValue::CONTINUE };
            if rule.block.unwrap_or(false) { return ReturnValue::CANCEL; }
            if let Some(to) = &rule.to {
                let new = rewrite_url(&rule.pattern, to, &url);
                req.set_url(Some(&CefString::from(new.as_str())));
            }
            ReturnValue::CONTINUE
        }

        fn resource_handler(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            request: Option<&mut Request>,
        ) -> Option<ResourceHandler> {
            let req = request?;
            let url = CefString::from(&req.url()).to_string();
            let rule = self.compiled.first_match(&url)?;
            if rule.pass.unwrap_or(false) || rule.block.unwrap_or(false) || rule.to.is_some() { return None; }
            if rule.status.is_none() && rule.body.is_none() { return None; }
            let bytes = rule.body.as_deref().unwrap_or("").as_bytes().to_vec();
            let status = u16::try_from(rule.status.unwrap_or(200)).unwrap_or(200);
            let headers = rule.headers.clone().unwrap_or_default();
            let mime = mime_of(&headers);
            Some(make_handler_with_headers(bytes, &mime, status, headers))
        }
    }
}

wrap_request_handler! {
    pub struct ProxyReqHandler { pub compiled: Arc<Compiled> }
    impl RequestHandler {
        fn resource_request_handler(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _request: Option<&mut Request>,
            _is_navigation: ::std::os::raw::c_int,
            _is_download: ::std::os::raw::c_int,
            _request_initiator: Option<&CefString>,
            _disable_default_handling: Option<&mut ::std::os::raw::c_int>,
        ) -> Option<ResourceRequestHandler> {
            Some(ProxyResReqHandler::new(self.compiled.clone()))
        }
    }
}
