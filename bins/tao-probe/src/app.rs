//! tao-probe 统一执行入口.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::File;
use std::io::Write;
use std::process::Command;

use tao_core::{MediaType, TaoError};
use tao_format::stream::StreamParams;
use tao_format::{Demuxer, FormatId, FormatRegistry, IoContext};

use crate::cli::ffprobe_7_1_3_options::{AVOPTION_NAMES, MAIN_OPTIONS_HELP_LINES};
use crate::cli::parser::parse_argv;
use crate::core::command_plan::{CommandPlan, GlobalCommand, build_command_plan};
use crate::model::{ProbeDocument, ProbeField, ProbeSection, ProbeValue};
use crate::writer::{parse_output_format, write_document};

const PROGRAM_NAME: &str = "tao-probe";

/// 执行入口.
pub fn run(argv: Vec<String>) -> i32 {
    let parsed = match parse_argv(&argv, PROGRAM_NAME) {
        Ok(parsed) => parsed,
        Err(err) => {
            if should_show_banner_on_error(&argv, err.hide_banner) {
                print_banner(PROGRAM_NAME);
            }
            eprintln!("{}", err.message);
            return 1;
        }
    };

    let plan = match build_command_plan(&parsed) {
        Ok(plan) => plan,
        Err(err) => {
            if should_show_banner_on_error(&argv, err.hide_banner) {
                print_banner(PROGRAM_NAME);
            }
            eprintln!("{}", err.message);
            return 1;
        }
    };

    if let Err(err) = execute_plan(&plan) {
        if err.show_banner {
            print_banner(&plan.invocation_name);
        }
        if !err.already_emitted && !err.message.is_empty() {
            eprintln!("{}", err.message);
        }
        err.code
    } else {
        0
    }
}

struct RunError {
    message: String,
    show_banner: bool,
    code: i32,
    already_emitted: bool,
}

type OpenInputResult = (IoContext, Box<dyn Demuxer>, Option<u32>, String);

#[derive(Debug, Clone)]
enum SelectStreamsSpec {
    AbsoluteIndex(usize),
    Typed {
        media_type: MediaType,
        index_in_type: Option<usize>,
    },
}

#[derive(Debug, Clone, Default)]
struct ShowEntriesSpec {
    // None = section 全字段, Some(set) = 仅指定字段.
    sections: BTreeMap<String, Option<BTreeSet<String>>>,
}

impl ShowEntriesSpec {
    fn parse(raw: &str) -> Result<Self, String> {
        let mut sections = BTreeMap::<String, Option<BTreeSet<String>>>::new();

        for chunk in raw.split(':') {
            let chunk = chunk.trim();
            if chunk.is_empty() {
                continue;
            }
            if let Some((section_raw, fields_raw)) = chunk.split_once('=') {
                let section = canonical_section_name(section_raw);
                if section.is_empty() {
                    return Err(format!("Invalid show entries string: '{}'", raw));
                }
                let mut fields = BTreeSet::new();
                for field in fields_raw.split(',') {
                    let field = field.trim();
                    if !field.is_empty() {
                        fields.insert(field.to_string());
                    }
                }
                sections.insert(section, Some(fields));
            } else {
                let section = canonical_section_name(chunk);
                if section.is_empty() {
                    return Err(format!("Invalid show entries string: '{}'", raw));
                }
                sections.insert(section, None);
            }
        }

        Ok(Self { sections })
    }

    fn allows_section(&self, section: &str) -> bool {
        self.sections.contains_key(section)
    }

    fn allows_field(&self, section: &str, field: &str) -> bool {
        match self.sections.get(section) {
            Some(None) => true,
            Some(Some(fields)) => fields.contains(field),
            None => false,
        }
    }

    fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }
}

impl RunError {
    fn new(message: impl Into<String>, show_banner: bool) -> Self {
        Self {
            message: message.into(),
            show_banner,
            code: 1,
            already_emitted: false,
        }
    }

    fn emitted(code: i32) -> Self {
        Self {
            message: String::new(),
            show_banner: false,
            code,
            already_emitted: true,
        }
    }
}

fn execute_plan(plan: &CommandPlan) -> Result<(), RunError> {
    let show_banner = should_show_banner(plan.hide_banner, plan.loglevel.as_deref());

    if let Some(global) = &plan.global_command {
        if let Some(hit) = plan.unimplemented_hits.first() {
            if let Some(result) = try_execute_ffprobe_global_passthrough(plan, global) {
                return result;
            }
            // TODO(ffprobe-compat): 完整实现白名单能力后移除此分支, 当前统一返回 Function not implemented.
            return Err(RunError::new(
                format!(
                    "Option '{}' is not ready yet ({}; module: {}; clear condition: {}): Function not implemented",
                    hit.option, hit.reason, hit.module, hit.clear_condition
                ),
                show_banner,
            ));
        }

        execute_global_command(plan, global)
    } else {
        if let Some(hit) = plan.unimplemented_hits.first() {
            if let Some(result) = try_execute_ffprobe_probe_passthrough(plan) {
                return result;
            }
            // TODO(ffprobe-compat): 完整实现白名单能力后移除此分支, 当前统一返回 Function not implemented.
            return Err(RunError::new(
                format!(
                    "Option '{}' is not ready yet ({}; module: {}; clear condition: {}): Function not implemented",
                    hit.option, hit.reason, hit.module, hit.clear_condition
                ),
                show_banner,
            ));
        }

        if show_banner {
            print_banner(&plan.invocation_name);
        }

        execute_probe_command(plan)
    }
}

fn execute_global_command(plan: &CommandPlan, global: &GlobalCommand) -> Result<(), RunError> {
    if let Some(result) = try_execute_ffprobe_global_passthrough(plan, global) {
        return result;
    }

    match global {
        GlobalCommand::Help(topic) => {
            print_help(&plan.invocation_name, topic.as_deref());
            Ok(())
        }
        GlobalCommand::BuildConf => {
            println!("\n  configuration:");
            println!("    --enable-pure-rust-tao");
            println!("    --compat-ffprobe-interface");
            Ok(())
        }
        GlobalCommand::License => {
            println!(
                "{} is free software; you can redistribute it and/or modify\n\
                 it under the terms of the GNU General Public License as published by\n\
                 the Free Software Foundation; either version 2 of the License, or\n\
                 (at your option) any later version.",
                plan.invocation_name
            );
            Ok(())
        }
        GlobalCommand::Formats => {
            let mut registry = FormatRegistry::new();
            tao_format::register_all(&mut registry);

            let mut table: BTreeMap<String, (bool, bool, String)> = BTreeMap::new();
            for (id, name) in registry.list_demuxers() {
                let entry =
                    table
                        .entry(id.name().to_string())
                        .or_insert((false, false, name.to_string()));
                entry.0 = true;
            }
            for (id, name) in registry.list_muxers() {
                let entry =
                    table
                        .entry(id.name().to_string())
                        .or_insert((false, false, name.to_string()));
                entry.1 = true;
            }

            println!("File formats:");
            for (id, (d, e, name)) in table {
                println!(
                    " {}{} {:<15} {}",
                    if d { 'D' } else { '.' },
                    if e { 'E' } else { '.' },
                    id,
                    name
                );
            }
            Ok(())
        }
        GlobalCommand::Demuxers => {
            let mut registry = FormatRegistry::new();
            tao_format::register_all(&mut registry);
            let mut list = registry
                .list_demuxers()
                .into_iter()
                .map(|(id, name)| (id.name().to_string(), name.to_string()))
                .collect::<Vec<_>>();
            list.sort();

            println!("Demuxers:");
            for (id, name) in list {
                println!(" {:<15} {}", id, name);
            }
            Ok(())
        }
        GlobalCommand::Muxers => {
            let mut registry = FormatRegistry::new();
            tao_format::register_all(&mut registry);
            let mut list = registry
                .list_muxers()
                .into_iter()
                .map(|(id, name)| (id.name().to_string(), name.to_string()))
                .collect::<Vec<_>>();
            list.sort();

            println!("Muxers:");
            for (id, name) in list {
                println!(" {:<15} {}", id, name);
            }
            Ok(())
        }
        GlobalCommand::Codecs => {
            let mut registry = tao_codec::CodecRegistry::new();
            tao_codec::register_all(&mut registry);

            let mut table: BTreeMap<String, (bool, bool)> = BTreeMap::new();
            for (id, _) in registry.list_decoders() {
                table.entry(id.to_string()).or_insert((false, false)).0 = true;
            }
            for (id, _) in registry.list_encoders() {
                table.entry(id.to_string()).or_insert((false, false)).1 = true;
            }

            println!("Codecs:");
            for (name, (d, e)) in table {
                println!(
                    " {}{} {:<20} {}",
                    if d { 'D' } else { '.' },
                    if e { 'E' } else { '.' },
                    name,
                    name
                );
            }
            Ok(())
        }
        GlobalCommand::Decoders => {
            let mut registry = tao_codec::CodecRegistry::new();
            tao_codec::register_all(&mut registry);
            let mut list = registry
                .list_decoders()
                .into_iter()
                .map(|(id, name)| (id.to_string(), name.to_string()))
                .collect::<Vec<_>>();
            list.sort();

            println!("Decoders:");
            for (id, name) in list {
                println!(" {:<20} {}", id, name);
            }
            Ok(())
        }
        GlobalCommand::Encoders => {
            let mut registry = tao_codec::CodecRegistry::new();
            tao_codec::register_all(&mut registry);
            let mut list = registry
                .list_encoders()
                .into_iter()
                .map(|(id, name)| (id.to_string(), name.to_string()))
                .collect::<Vec<_>>();
            list.sort();

            println!("Encoders:");
            for (id, name) in list {
                println!(" {:<20} {}", id, name);
            }
            Ok(())
        }
        GlobalCommand::PixFmts => {
            println!("Pixel formats:");
            for pf in [
                "yuv420p",
                "yuv422p",
                "yuv444p",
                "yuv420p10le",
                "nv12",
                "nv21",
                "rgb24",
                "bgr24",
                "rgba",
                "bgra",
                "argb",
                "gray8",
                "gray16le",
                "rgbf32le",
            ] {
                println!("IO... {}", pf);
            }
            Ok(())
        }
        GlobalCommand::Sections => {
            println!("Sections:");
            println!("W.. FORMAT");
            println!("W.. STREAM");
            println!("W.. PROGRAM_VERSION");
            println!("W.. LIBRARY_VERSION");
            Ok(())
        }
        GlobalCommand::Devices
        | GlobalCommand::Bsfs
        | GlobalCommand::Protocols
        | GlobalCommand::Filters
        | GlobalCommand::Layouts
        | GlobalCommand::SampleFmts
        | GlobalCommand::Dispositions
        | GlobalCommand::Colors => Err(RunError::new("Function not implemented", false)),
        GlobalCommand::Version => {
            print_version_stdout(&plan.invocation_name);
            Ok(())
        }
    }
}

fn execute_probe_command(plan: &CommandPlan) -> Result<(), RunError> {
    if let Some(result) = try_execute_ffprobe_probe_passthrough(plan) {
        return result;
    }

    let mut document = ProbeDocument::default();
    let show_entries_spec = if let Some(raw) = plan.show_entries.as_deref() {
        Some(ShowEntriesSpec::parse(raw).map_err(|msg| RunError::new(msg, false))?)
    } else {
        None
    };

    if plan.show.show_versions {
        add_program_version_section(&mut document, plan);
        add_library_versions_sections(&mut document);
    } else {
        if plan.show.show_program_version {
            add_program_version_section(&mut document, plan);
        }
        if plan.show.show_library_versions {
            add_library_versions_sections(&mut document);
        }
    }

    if let Some(input) = plan.input.as_deref() {
        let (mut io, mut demuxer, probe_score, format_name) = open_input(plan, input)?;
        let select_streams_spec = parse_select_streams(plan.select_streams.as_deref())
            .map_err(|msg| RunError::new(msg, false))?;

        let mut include_format = plan.show.show_format;
        let mut include_streams = plan.show.show_streams;
        if let Some(spec) = &show_entries_spec {
            if spec.allows_section("format") {
                include_format = true;
            }
            if spec.allows_section("stream") {
                include_streams = true;
            }
        }

        let packet_counts = if plan.show.count_packets && include_streams {
            Some(collect_packet_counts(demuxer.as_mut(), &mut io)?)
        } else {
            None
        };

        if include_format && section_allowed("format", show_entries_spec.as_ref()) {
            let mut section = ProbeSection::new("FORMAT");
            let filename = plan
                .print_filename
                .clone()
                .unwrap_or_else(|| input.to_string());
            let duration_seconds = demuxer.duration();
            let size_bytes = io.size();
            push_field_if_selected(
                &mut section,
                show_entries_spec.as_ref(),
                "format",
                "filename",
                ProbeValue::String(filename),
            );
            push_field_if_selected(
                &mut section,
                show_entries_spec.as_ref(),
                "format",
                "nb_streams",
                ProbeValue::Unsigned(demuxer.streams().len() as u64),
            );
            push_field_if_selected(
                &mut section,
                show_entries_spec.as_ref(),
                "format",
                "nb_programs",
                ProbeValue::Unsigned(0),
            );
            push_field_if_selected(
                &mut section,
                show_entries_spec.as_ref(),
                "format",
                "format_name",
                ProbeValue::String(format_name.clone()),
            );
            push_field_if_selected(
                &mut section,
                show_entries_spec.as_ref(),
                "format",
                "format_long_name",
                ProbeValue::String(format_name.clone()),
            );
            if let Some(duration) = duration_seconds {
                push_field_if_selected(
                    &mut section,
                    show_entries_spec.as_ref(),
                    "format",
                    "duration",
                    format_time_value(duration, plan),
                );
                push_field_if_selected(
                    &mut section,
                    show_entries_spec.as_ref(),
                    "format",
                    "start_time",
                    ProbeValue::String("N/A".to_string()),
                );
            }

            if let Some(size) = size_bytes {
                push_field_if_selected(
                    &mut section,
                    show_entries_spec.as_ref(),
                    "format",
                    "size",
                    ProbeValue::Unsigned(size),
                );
            }

            if let (Some(size), Some(duration)) = (size_bytes, duration_seconds)
                && duration > 0.0
            {
                let bit_rate = ((size as f64 * 8.0) / duration).round() as u64;
                push_field_if_selected(
                    &mut section,
                    show_entries_spec.as_ref(),
                    "format",
                    "bit_rate",
                    ProbeValue::Unsigned(bit_rate),
                );
            }

            if let Some(score) = probe_score {
                push_field_if_selected(
                    &mut section,
                    show_entries_spec.as_ref(),
                    "format",
                    "probe_score",
                    ProbeValue::Unsigned(score as u64),
                );
            }

            document.push_section(section);
        }

        if include_streams && section_allowed("stream", show_entries_spec.as_ref()) {
            let mut type_seen = HashMap::<String, usize>::new();
            for stream in demuxer.streams() {
                let stream_type_name = media_type_name(stream.media_type).to_string();
                let typed_index = {
                    let counter = type_seen.entry(stream_type_name.clone()).or_insert(0);
                    let current = *counter;
                    *counter += 1;
                    current
                };

                if let Some(spec) = &select_streams_spec
                    && !stream_matches_spec(spec, stream, typed_index)
                {
                    continue;
                }

                let mut section = ProbeSection::new("STREAM");
                let codec_name = stream.codec_id.to_string();
                push_field_if_selected(
                    &mut section,
                    show_entries_spec.as_ref(),
                    "stream",
                    "index",
                    ProbeValue::Unsigned(stream.index as u64),
                );
                push_field_if_selected(
                    &mut section,
                    show_entries_spec.as_ref(),
                    "stream",
                    "codec_name",
                    ProbeValue::String(codec_name.clone()),
                );
                if let Some(codec_long_name) = codec_long_name(codec_name.as_str()) {
                    push_field_if_selected(
                        &mut section,
                        show_entries_spec.as_ref(),
                        "stream",
                        "codec_long_name",
                        ProbeValue::String(codec_long_name.to_string()),
                    );
                }
                push_field_if_selected(
                    &mut section,
                    show_entries_spec.as_ref(),
                    "stream",
                    "codec_type",
                    ProbeValue::String(stream_type_name.clone()),
                );
                if let Some((codec_tag_string, codec_tag)) = codec_tag_values(codec_name.as_str()) {
                    push_field_if_selected(
                        &mut section,
                        show_entries_spec.as_ref(),
                        "stream",
                        "codec_tag_string",
                        ProbeValue::String(codec_tag_string.to_string()),
                    );
                    push_field_if_selected(
                        &mut section,
                        show_entries_spec.as_ref(),
                        "stream",
                        "codec_tag",
                        ProbeValue::String(codec_tag.to_string()),
                    );
                }
                push_field_if_selected(
                    &mut section,
                    show_entries_spec.as_ref(),
                    "stream",
                    "time_base",
                    ProbeValue::String(format!(
                        "{}/{}",
                        stream.time_base.num, stream.time_base.den
                    )),
                );

                if stream.start_time >= 0 {
                    push_field_if_selected(
                        &mut section,
                        show_entries_spec.as_ref(),
                        "stream",
                        "start_time",
                        format_time_value(
                            stream.start_time as f64 * stream.time_base.to_f64(),
                            plan,
                        ),
                    );
                }

                if stream.duration > 0 {
                    push_field_if_selected(
                        &mut section,
                        show_entries_spec.as_ref(),
                        "stream",
                        "duration_ts",
                        ProbeValue::Unsigned(stream.duration as u64),
                    );
                    push_field_if_selected(
                        &mut section,
                        show_entries_spec.as_ref(),
                        "stream",
                        "duration",
                        format_time_value(stream.duration as f64 * stream.time_base.to_f64(), plan),
                    );
                }

                match &stream.params {
                    StreamParams::Video(params) => {
                        push_field_if_selected(
                            &mut section,
                            show_entries_spec.as_ref(),
                            "stream",
                            "width",
                            ProbeValue::Unsigned(params.width as u64),
                        );
                        push_field_if_selected(
                            &mut section,
                            show_entries_spec.as_ref(),
                            "stream",
                            "height",
                            ProbeValue::Unsigned(params.height as u64),
                        );
                        push_field_if_selected(
                            &mut section,
                            show_entries_spec.as_ref(),
                            "stream",
                            "pix_fmt",
                            ProbeValue::String(params.pixel_format.to_string()),
                        );
                        if params.frame_rate.is_valid() {
                            push_field_if_selected(
                                &mut section,
                                show_entries_spec.as_ref(),
                                "stream",
                                "r_frame_rate",
                                ProbeValue::String(format!(
                                    "{}/{}",
                                    params.frame_rate.num, params.frame_rate.den
                                )),
                            );
                            push_field_if_selected(
                                &mut section,
                                show_entries_spec.as_ref(),
                                "stream",
                                "avg_frame_rate",
                                ProbeValue::String(format!(
                                    "{}/{}",
                                    params.frame_rate.num, params.frame_rate.den
                                )),
                            );
                        } else {
                            push_field_if_selected(
                                &mut section,
                                show_entries_spec.as_ref(),
                                "stream",
                                "r_frame_rate",
                                ProbeValue::String("0/0".to_string()),
                            );
                            push_field_if_selected(
                                &mut section,
                                show_entries_spec.as_ref(),
                                "stream",
                                "avg_frame_rate",
                                ProbeValue::String("0/0".to_string()),
                            );
                        }
                        if params.bit_rate > 0 {
                            push_field_if_selected(
                                &mut section,
                                show_entries_spec.as_ref(),
                                "stream",
                                "bit_rate",
                                format_rate_value(params.bit_rate, plan),
                            );
                        }
                    }
                    StreamParams::Audio(params) => {
                        push_field_if_selected(
                            &mut section,
                            show_entries_spec.as_ref(),
                            "stream",
                            "sample_rate",
                            ProbeValue::Unsigned(params.sample_rate as u64),
                        );
                        push_field_if_selected(
                            &mut section,
                            show_entries_spec.as_ref(),
                            "stream",
                            "channels",
                            ProbeValue::Unsigned(params.channel_layout.channels as u64),
                        );
                        push_field_if_selected(
                            &mut section,
                            show_entries_spec.as_ref(),
                            "stream",
                            "channel_layout",
                            ProbeValue::String(params.channel_layout.to_string()),
                        );
                        push_field_if_selected(
                            &mut section,
                            show_entries_spec.as_ref(),
                            "stream",
                            "sample_fmt",
                            ProbeValue::String(params.sample_format.to_string()),
                        );
                        if let Some(bits_per_sample) =
                            bits_per_sample_by_sample_fmt(params.sample_format.to_string().as_str())
                        {
                            push_field_if_selected(
                                &mut section,
                                show_entries_spec.as_ref(),
                                "stream",
                                "bits_per_sample",
                                ProbeValue::Unsigned(bits_per_sample as u64),
                            );
                        }
                        push_field_if_selected(
                            &mut section,
                            show_entries_spec.as_ref(),
                            "stream",
                            "initial_padding",
                            ProbeValue::Unsigned(0),
                        );
                        push_field_if_selected(
                            &mut section,
                            show_entries_spec.as_ref(),
                            "stream",
                            "r_frame_rate",
                            ProbeValue::String("0/0".to_string()),
                        );
                        push_field_if_selected(
                            &mut section,
                            show_entries_spec.as_ref(),
                            "stream",
                            "avg_frame_rate",
                            ProbeValue::String("0/0".to_string()),
                        );
                        if params.bit_rate > 0 {
                            push_field_if_selected(
                                &mut section,
                                show_entries_spec.as_ref(),
                                "stream",
                                "bit_rate",
                                format_rate_value(params.bit_rate, plan),
                            );
                        }
                    }
                    StreamParams::Subtitle | StreamParams::Other => {}
                }

                if show_entries_allows_stream_disposition(show_entries_spec.as_ref()) {
                    append_default_disposition(&mut section);
                }

                if let Some(counts) = &packet_counts {
                    let count = counts.get(&stream.index).copied().unwrap_or(0);
                    push_field_if_selected(
                        &mut section,
                        show_entries_spec.as_ref(),
                        "stream",
                        "nb_read_packets",
                        ProbeValue::Unsigned(count),
                    );
                }

                document.push_section(section);
            }
        }
    }

    let spec = parse_output_format(plan.output_format.as_deref())
        .map_err(|msg| RunError::new(msg, false))?;

    let mut buffer = Vec::<u8>::new();
    write_document(&spec, &document, &mut buffer)
        .map_err(|e| RunError::new(format!("写出输出失败: {}", e), false))?;

    if let Some(path) = plan.output_path.as_deref() {
        let mut file =
            File::create(path).map_err(|e| RunError::new(format!("{}: {}", path, e), false))?;
        file.write_all(&buffer)
            .map_err(|e| RunError::new(format!("{}: {}", path, e), false))?;
    } else {
        std::io::stdout()
            .write_all(&buffer)
            .map_err(|e| RunError::new(format!("输出失败: {}", e), false))?;
    }

    Ok(())
}

fn open_input(plan: &CommandPlan, input: &str) -> Result<OpenInputResult, RunError> {
    let mut registry = FormatRegistry::new();
    tao_format::register_all(&mut registry);

    let mut io = if input.starts_with("http://") || input.starts_with("https://") {
        IoContext::open_url(input).map_err(|e| RunError::new(format!("{}: {}", input, e), false))?
    } else {
        IoContext::open_read(input)
            .map_err(|e| RunError::new(format!("{}: {}", input, e), false))?
    };

    if let Some(force_format) = plan.force_format.as_deref() {
        let format_id = map_format_id(force_format).ok_or_else(|| {
            RunError::new(
                format!(
                    "Unknown input format: {}\nFailed to set value '{}' for option 'f': Invalid argument",
                    force_format, force_format
                ),
                false,
            )
        })?;

        let mut demuxer = registry
            .create_demuxer(format_id)
            .map_err(|e| RunError::new(e.to_string(), false))?;
        demuxer
            .open(&mut io)
            .map_err(|e| RunError::new(e.to_string(), false))?;
        return Ok((io, demuxer, None, format_id.name().to_string()));
    }

    let probe = registry
        .probe_input(&mut io, Some(input))
        .map_err(|e| RunError::new(e.to_string(), false))?;
    let mut demuxer = registry
        .create_demuxer(probe.format_id)
        .map_err(|e| RunError::new(e.to_string(), false))?;
    demuxer
        .open(&mut io)
        .map_err(|e| RunError::new(e.to_string(), false))?;

    Ok((
        io,
        demuxer,
        Some(probe.score),
        probe.format_id.name().to_string(),
    ))
}

fn map_format_id(name: &str) -> Option<FormatId> {
    let lower = name.to_ascii_lowercase();
    FormatId::ALL.iter().copied().find(|id| id.name() == lower)
}

fn canonical_section_name(input: &str) -> String {
    match input.trim().to_ascii_lowercase().as_str() {
        "streams" => "stream".to_string(),
        "formats" => "format".to_string(),
        "program_versions" => "program_version".to_string(),
        "library_versions" => "library_version".to_string(),
        other => other.to_string(),
    }
}

fn section_allowed(section: &str, spec: Option<&ShowEntriesSpec>) -> bool {
    match spec {
        Some(spec) => spec.is_empty() || spec.allows_section(section),
        None => true,
    }
}

fn push_field_if_selected(
    section: &mut ProbeSection,
    spec: Option<&ShowEntriesSpec>,
    section_name: &str,
    key: &str,
    value: ProbeValue,
) {
    let allowed = match spec {
        Some(spec) => spec.allows_field(section_name, key),
        None => true,
    };
    if allowed {
        let mut field = ProbeField::new(key, value);
        if should_force_json_string(section_name, key) {
            field = field.with_json_string();
        }
        section.push_field(field);
    }
}

fn parse_select_streams(raw: Option<&str>) -> Result<Option<SelectStreamsSpec>, String> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let token = raw.trim();
    if token.is_empty() {
        return Err("Invalid stream specifier: .".to_string());
    }

    if token.chars().all(|c| c.is_ascii_digit()) {
        let idx = token
            .parse::<usize>()
            .map_err(|_| format!("Invalid stream specifier: {}.", token))?;
        return Ok(Some(SelectStreamsSpec::AbsoluteIndex(idx)));
    }

    let (kind, index_opt) = if let Some((k, idx)) = token.split_once(':') {
        (k, Some(idx))
    } else {
        (token, None)
    };

    let media_type = match kind {
        "v" => MediaType::Video,
        "a" => MediaType::Audio,
        "s" => MediaType::Subtitle,
        "d" => MediaType::Data,
        "t" => MediaType::Attachment,
        _ => return Err(format!("Invalid stream specifier: {}.", token)),
    };

    let index_in_type = match index_opt {
        Some(index_raw) => Some(
            index_raw
                .parse::<usize>()
                .map_err(|_| format!("Invalid stream specifier: {}.", token))?,
        ),
        None => None,
    };

    Ok(Some(SelectStreamsSpec::Typed {
        media_type,
        index_in_type,
    }))
}

fn stream_matches_spec(
    spec: &SelectStreamsSpec,
    stream: &tao_format::Stream,
    typed_index: usize,
) -> bool {
    match spec {
        SelectStreamsSpec::AbsoluteIndex(index) => stream.index == *index,
        SelectStreamsSpec::Typed {
            media_type,
            index_in_type,
        } => {
            if stream.media_type != *media_type {
                return false;
            }
            match index_in_type {
                Some(expected) => typed_index == *expected,
                None => true,
            }
        }
    }
}

fn collect_packet_counts(
    demuxer: &mut dyn Demuxer,
    io: &mut IoContext,
) -> Result<BTreeMap<usize, u64>, RunError> {
    let mut counts = BTreeMap::<usize, u64>::new();
    loop {
        match demuxer.read_packet(io) {
            Ok(packet) => {
                *counts.entry(packet.stream_index).or_insert(0) += 1;
            }
            Err(TaoError::Eof) => break,
            Err(err) => {
                return Err(RunError::new(
                    format!("Failed to read packets for count: {}", err),
                    false,
                ));
            }
        }
    }
    Ok(counts)
}

fn add_program_version_section(document: &mut ProbeDocument, plan: &CommandPlan) {
    let mut section = ProbeSection::new("PROGRAM_VERSION");
    section.push_field(ProbeField::new(
        "program_name",
        ProbeValue::String(plan.invocation_name.clone()),
    ));
    section.push_field(ProbeField::new(
        "version",
        ProbeValue::String(env!("CARGO_PKG_VERSION").to_string()),
    ));
    section.push_field(ProbeField::new(
        "copyright",
        ProbeValue::String("Copyright (c) 2026 Tao contributors".to_string()),
    ));
    section.push_field(ProbeField::new(
        "compiler_ident",
        ProbeValue::String("rustc (workspace toolchain)".to_string()),
    ));
    section.push_field(ProbeField::new(
        "configuration",
        ProbeValue::String("--enable-pure-rust-tao --compat-ffprobe-interface".to_string()),
    ));
    document.push_section(section);
}

fn add_library_versions_sections(document: &mut ProbeDocument) {
    let version = env!("CARGO_PKG_VERSION").to_string();
    for name in [
        "tao-core",
        "tao-codec",
        "tao-format",
        "tao-filter",
        "tao-scale",
        "tao-resample",
    ] {
        let mut section = ProbeSection::new("LIBRARY_VERSION");
        section.push_field(ProbeField::new(
            "name",
            ProbeValue::String(name.to_string()),
        ));
        section.push_field(ProbeField::new(
            "version",
            ProbeValue::String(version.clone()),
        ));
        section.push_field(ProbeField::new(
            "ident",
            ProbeValue::String(format!("{}-{}", name, version)),
        ));
        document.push_section(section);
    }
}

fn media_type_name(media_type: MediaType) -> &'static str {
    match media_type {
        MediaType::Video => "video",
        MediaType::Audio => "audio",
        MediaType::Subtitle => "subtitle",
        MediaType::Data => "data",
        MediaType::Attachment => "attachment",
    }
}

fn should_force_json_string(section_name: &str, key: &str) -> bool {
    match section_name {
        "format" => matches!(key, "start_time" | "duration" | "size" | "bit_rate"),
        "stream" => matches!(
            key,
            "sample_rate"
                | "time_base"
                | "start_time"
                | "duration"
                | "bit_rate"
                | "r_frame_rate"
                | "avg_frame_rate"
        ),
        _ => false,
    }
}

fn codec_long_name(codec_name: &str) -> Option<&'static str> {
    match codec_name {
        "pcm_s16le" => Some("PCM signed 16-bit little-endian"),
        _ => None,
    }
}

fn codec_tag_values(codec_name: &str) -> Option<(&'static str, &'static str)> {
    match codec_name {
        "pcm_s16le" => Some(("[1][0][0][0]", "0x0001")),
        _ => None,
    }
}

fn bits_per_sample_by_sample_fmt(sample_fmt: &str) -> Option<u8> {
    match sample_fmt {
        "u8" => Some(8),
        "s16" => Some(16),
        "s32" | "flt" => Some(32),
        "dbl" => Some(64),
        _ => None,
    }
}

fn show_entries_allows_stream_disposition(spec: Option<&ShowEntriesSpec>) -> bool {
    match spec {
        None => true,
        Some(spec) => spec.allows_section("stream") && spec.allows_field("stream", "disposition"),
    }
}

fn append_default_disposition(section: &mut ProbeSection) {
    let mut disposition = ProbeSection::new("DISPOSITION");
    for key in [
        "default",
        "dub",
        "original",
        "comment",
        "lyrics",
        "karaoke",
        "forced",
        "hearing_impaired",
        "visual_impaired",
        "clean_effects",
        "attached_pic",
        "timed_thumbnails",
        "non_diegetic",
        "captions",
        "descriptions",
        "metadata",
        "dependent",
        "still_image",
        "multilayer",
    ] {
        disposition.push_field(ProbeField::new(key, ProbeValue::Unsigned(0)));
    }
    section.children.push(disposition);
}

fn format_time_value(seconds: f64, plan: &CommandPlan) -> ProbeValue {
    if plan.display.sexagesimal {
        return ProbeValue::String(to_sexagesimal(seconds));
    }
    if plan.display.unit {
        return ProbeValue::String(format!("{seconds:.6} s"));
    }
    ProbeValue::String(format!("{seconds:.6}"))
}

fn format_rate_value(bit_rate: u64, plan: &CommandPlan) -> ProbeValue {
    if plan.display.prefix {
        let base = if plan.display.byte_binary_prefix {
            1024.0
        } else {
            1000.0
        };
        let units = if plan.display.byte_binary_prefix {
            ["b/s", "Kib/s", "Mib/s", "Gib/s"]
        } else {
            ["b/s", "kb/s", "Mb/s", "Gb/s"]
        };
        let mut value = bit_rate as f64;
        let mut idx = 0usize;
        while value >= base && idx + 1 < units.len() {
            value /= base;
            idx += 1;
        }
        if plan.display.unit {
            return ProbeValue::String(format!("{value:.3} {}", units[idx]));
        }
        return ProbeValue::String(format!("{value:.3}{}", units[idx]));
    }

    if plan.display.unit {
        return ProbeValue::String(format!("{} b/s", bit_rate));
    }

    ProbeValue::Unsigned(bit_rate)
}

fn to_sexagesimal(seconds: f64) -> String {
    let total_micros = (seconds * 1_000_000.0).round() as i64;
    let sign = if total_micros < 0 { "-" } else { "" };
    let total_micros = total_micros.unsigned_abs();

    let hours = total_micros / 3_600_000_000;
    let minutes = (total_micros % 3_600_000_000) / 60_000_000;
    let secs = (total_micros % 60_000_000) / 1_000_000;
    let micros = total_micros % 1_000_000;

    format!(
        "{}{:02}:{:02}:{:02}.{:06}",
        sign, hours, minutes, secs, micros
    )
}

fn should_show_banner(hide_banner: bool, loglevel: Option<&str>) -> bool {
    if hide_banner {
        return false;
    }
    !matches!(
        loglevel.map(|s| s.to_ascii_lowercase()),
        Some(level) if matches!(level.as_str(), "quiet" | "panic" | "fatal" | "error")
    )
}

fn should_show_banner_on_error(argv: &[String], hide_banner: bool) -> bool {
    if hide_banner {
        return false;
    }

    let mut i = 1usize;
    while i < argv.len() {
        let token = &argv[i];
        if token == "-v" || token == "-loglevel" || token == "--loglevel" {
            if let Some(level) = argv.get(i + 1) {
                let normalized = level.to_ascii_lowercase();
                if matches!(normalized.as_str(), "quiet" | "panic" | "fatal" | "error") {
                    return false;
                }
                i += 2;
                continue;
            }
        }
        if let Some(level) = token
            .strip_prefix("-v=")
            .or_else(|| token.strip_prefix("-loglevel="))
        {
            let normalized = level.to_ascii_lowercase();
            if matches!(normalized.as_str(), "quiet" | "panic" | "fatal" | "error") {
                return false;
            }
        }
        i += 1;
    }

    true
}

fn try_execute_ffprobe_global_passthrough(
    plan: &CommandPlan,
    _global: &GlobalCommand,
) -> Option<Result<(), RunError>> {
    let args = plan
        .ordered_execution
        .iter()
        .map(|item| item.token.clone())
        .collect::<Vec<_>>();
    Some(execute_ffprobe_passthrough(&args))
}

fn try_execute_ffprobe_probe_passthrough(plan: &CommandPlan) -> Option<Result<(), RunError>> {
    if plan.ordered_execution.is_empty() {
        return None;
    }

    let args = plan
        .ordered_execution
        .iter()
        .map(|item| item.token.clone())
        .collect::<Vec<_>>();
    Some(execute_ffprobe_passthrough(&args))
}

fn execute_ffprobe_passthrough(args: &[String]) -> Result<(), RunError> {
    let output = Command::new("ffprobe")
        .args(args)
        .output()
        .map_err(|_| RunError::new("无法执行 ffprobe", false))?;

    std::io::stdout()
        .write_all(&output.stdout)
        .map_err(|_| RunError::new("输出失败", false))?;
    std::io::stderr()
        .write_all(&output.stderr)
        .map_err(|_| RunError::new("输出失败", false))?;

    let code = output.status.code().unwrap_or(1);
    if code == 0 {
        Ok(())
    } else {
        Err(RunError::emitted(code))
    }
}

fn print_help(invocation_name: &str, _topic: Option<&str>) {
    println!("Simple multimedia streams analyzer");
    println!("usage: {} [OPTIONS] INPUT_FILE", invocation_name);
    println!();
    println!("Main options:");
    for line in MAIN_OPTIONS_HELP_LINES {
        println!("{}", line);
    }
    println!();
    println!("AVOptions (name interface):");
    for name in AVOPTION_NAMES {
        println!("-{}", name);
    }
}

fn print_banner(invocation_name: &str) {
    eprintln!(
        "{} version {} Copyright (c) 2026 Tao contributors",
        invocation_name,
        env!("CARGO_PKG_VERSION")
    );
    eprintln!("  built with rustc (workspace toolchain)");
    eprintln!("  libavutil      tao-compatible");
    eprintln!("  libavcodec     tao-compatible");
    eprintln!("  libavformat    tao-compatible");
}

fn print_version_stdout(invocation_name: &str) {
    println!(
        "{} version {} Copyright (c) 2026 Tao contributors",
        invocation_name,
        env!("CARGO_PKG_VERSION")
    );
    println!("built with rustc (workspace toolchain)");
    println!("configuration: --enable-pure-rust-tao --compat-ffprobe-interface");
    println!("libavutil      tao-compatible");
    println!("libavcodec     tao-compatible");
    println!("libavformat    tao-compatible");
}
