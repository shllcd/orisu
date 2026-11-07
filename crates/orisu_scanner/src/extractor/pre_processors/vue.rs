use super::pre_process;
use bstr::ByteSlice;
use regex::Regex;
use std::sync;

static TEMPLATE_REGEX: sync::LazyLock<Regex> =
    sync::LazyLock::new(|| Regex::new(r#"<template lang=['"]([^"']*)['"]>([\s\S]*)<\/template>"#).unwrap());

pub fn process(content: &[u8]) -> Vec<u8> {
    let mut result = content.to_vec();

    if let Ok(content_as_str) = std::str::from_utf8(content) {
        for (_, [lang, body]) in TEMPLATE_REGEX.captures_iter(content_as_str).map(|c| c.extract()) {
            let replaced = pre_process(body.as_bytes().to_vec(), lang);
            result = result.replace(body, replaced);
        }
    }

    result
}
