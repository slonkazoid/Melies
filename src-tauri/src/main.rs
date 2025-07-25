#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use addons::compile_addons;
use chrono::prelude::*;
use regex::Regex;
use sanitize_filename::sanitize;
use serde_json::{self, json, Value};
use std::env::{current_dir, home_dir};
use std::ffi::OsStr;
use std::fs::{DirEntry, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::{env, fs};
use tauri::command;
use vdm::action::ActionType;
use vdm::VDM;

use crate::clip::Clip;
use crate::demos::*;
use crate::event::Event;
use crate::event::EventStyle::{Bookmark, Killstreak};
use crate::logstf::parse;
use crate::macros::*;
use crate::vdms::*;

mod addons;
mod clip;
mod demos;
mod event;
mod logstf;
mod macros;
mod settings;
mod tf2;
mod vdms;

fn setting_as_bin(setting: &Value) -> i64 {
    if !setting.is_boolean() {
        if let Some(setting_i64) = setting.as_i64() {
            return setting_i64;
        }
    }

    match setting.as_bool() {
        Some(val) => match val {
            true => 1,
            false => 0,
        },
        _ => 0,
    }
}

fn setting_as_bool(setting: &Value) -> bool {
    if setting.is_boolean() {
        return match setting.as_bool() {
            Some(val) => val,
            None => false,
        };
    }

    match setting.as_i64() {
        Some(val) => match val {
            1 => true,
            _ => false,
        },
        _ => false,
    }
}

fn write_cfg(settings: &Value) -> Result<(), String> {
    let mut cfg = String::new();

    extend!(
        cfg,
        "echo \"Execing Melies Config\";\ncl_drawhud {};\n",
        setting_as_bin(&settings["output"]["hud"])
    );
    extend!(cfg, "sv_cheats {};\n", "1");
    extend!(
        cfg,
        "voice_enable {};\n",
        setting_as_bin(&settings["output"]["voice_chat"])
    );
    extend!(
        cfg,
        "hud_saytext_time {};\n",
        setting_as_bin(&settings["output"]["text_chat"]) * 12
    );
    extend!(
        cfg,
        "crosshair {};\n",
        setting_as_bin(&settings["output"]["crosshair"])
    );
    extend!(
        cfg,
        "r_drawviewmodel {};\n",
        setting_as_bin(&settings["output"]["viewmodel"])
    );
    extend!(
        cfg,
        "tf_use_min_viewmodels {};\n",
        setting_as_bin(&settings["output"]["minmode"])
    );
    extend!(
        cfg,
        "viewmodel_fov_demo {};\n",
        setting_as_bin(&settings["recording"]["viewmodel_fov"])
    );
    extend!(
        cfg,
        "fov_desired {};\n",
        setting_as_bin(&settings["recording"]["fov"])
    );

    if setting_as_bin(&settings["recording"]["third_person"]) == 1 {
        extend!(cfg, "thirdperson{};\n", "");
    } else {
        extend!(cfg, "firstperson{};\n", "");
    }

    if let Some(commands) = settings["recording"]["commands"].as_str() {
        extend!(cfg, "{}\n", commands);
    }

    extend!(
        cfg,
        "{};\n",
        "alias \"snd_fix\" \"snd_restart; snd_soundmixer Default_mix;\""
    );

    extend!(cfg, "{}", compile_addons(settings));

    let tf_folder = match settings["tf_folder"].as_str() {
        Some(folder) => folder,
        None => return Err("tf_folder setting is not a string".to_string()),
    };

    let base_folder = Path::new(tf_folder).parent().unwrap();

    let mut installs = vec![PathBuf::from("tf")];

    for install in settings["alt_installs"].as_array().unwrap() {
        if let None = install["name"].as_str() {
            continue;
        }

        if let None = install["tf_folder"].as_str() {
            continue;
        }

        let install_folder = Path::new(install["tf_folder"].as_str().unwrap())
            .strip_prefix(base_folder)
            .unwrap();

        installs.push(install_folder.to_owned());
    }

    for install in installs {
        let install_folder = base_folder.join(install);

        let cfg_path = install_folder.join("cfg");

        if !Path::new(&cfg_path).exists() {
            match std::fs::create_dir_all(Path::new(&cfg_path)) {
                Ok(_) => {}
                Err(why) => return Err(format!("Couldn't create cfg folder: {}", why)),
            }
        }

        let mut file = match File::create(cfg_path.join("melies.cfg")) {
            Ok(file) => file,
            Err(why) => return Err(format!("Couldn't create melies.cfg: {}", why)),
        };

        match file.write_all(cfg.as_bytes()) {
            Ok(_) => {}
            Err(why) => return Err(format!("Couldn't write melies.cfg: {}", why)),
        };
    }

    Ok(())
}

fn end_vdm(vdm: &mut VDM, settings: &Value, next_demoname: String) -> VDM {
    let last_tick = match vdm.last().props().start_tick {
        Some(tick) => tick,
        None => {
            return vdm.to_owned();
        }
    };

    {
        let exec_commands = vdm.create_action(ActionType::PlayCommands).props_mut();

        exec_commands.start_tick = Some(last_tick + 66);

        if setting_as_bool(&settings["recording"]["record_continuous"]) && next_demoname != "" {
            exec_commands.name = format!("Start the next demo ({}.dem)", next_demoname);
            exec_commands.commands = format!("playdemo {};", next_demoname);
            return vdm.to_owned();
        }

        exec_commands.name = "Exit TF2".to_string();
        exec_commands.commands = "quit;".to_string();
    }

    vdm.to_owned()
}

fn check_spec(clip: &Clip, commands: String) -> String {
    let settings = load_settings();

    if clip.spec_type == 0 {
        return commands;
    }

    let mut new_commands = commands;

    let use_ce_spec: bool = match settings["recording"]["use_ce_spec"].as_bool() {
        Some(val) => val,
        None => false,
    };

    new_commands = format!(
        "{}; {} {}; spec_mode {};",
        new_commands,
        ifelse!(use_ce_spec, "ce_cameratools_spec_steamid", "spec_player"),
        clip.spec_player,
        ifelse!(clip.spec_type == 1, 4, 5)
    );

    return new_commands;
}

fn start_vdm(vdm: &mut VDM, clip: &Clip, settings: &Value) {
    if let Some(start_delay) = settings["recording"]["start_delay"].as_i64() {
        if clip.start_tick > start_delay + 66 {
            let skip_props = vdm.create_action(ActionType::SkipAhead).props_mut();

            if let Some(start_delay) = settings["recording"]["start_delay"].as_i64() {
                skip_props.start_tick = Some(start_delay);
            }
            if let Some(skip_to_tick) = clip.start_tick.checked_sub(66) {
                skip_props.skip_to_tick = Some(skip_to_tick);
            }

            skip_props.name = format!("Skip to first clip");
        }
    }

    record_clip(vdm, clip, settings);
}

fn add_clip_to_vdm(vdm: &mut VDM, clip: &Clip, settings: &Value) {
    let last_tick = match vdm.last().props().start_tick {
        Some(action) => action,
        None => {
            return;
        }
    };

    if clip.start_tick > last_tick + 300 {
        let skip_props = vdm.create_action(ActionType::SkipAhead).props_mut();

        skip_props.name = format!("Skip {} ticks", clip.start_tick - last_tick);
        skip_props.start_tick = Some(last_tick + 66);
        skip_props.skip_to_tick = Some(clip.start_tick - 66);
    }

    record_clip(vdm, clip, settings);
}

fn record_clip(vdm: &mut VDM, clip: &Clip, settings: &Value) {
    let mut vdm_name = vdm.name.clone();

    if settings["absolute_file_paths"].as_bool().unwrap_or(false) {
        vdm_name = vdm_name.replace("demos\\", "");
    }

    let mut suffix = "bm".to_string();

    if clip.has_killstreak {
        suffix = format!("ks{}", clip.ks_value);

        if clip.has_bookmark {
            suffix = format!("bm{}+", clip.ks_value);
        }
    }

    {
        let exec_commands = vdm.create_action(ActionType::PlayCommands).props_mut();

        exec_commands.start_tick = Some(clip.start_tick - 33);
        exec_commands.name = "Exec Melies Commands".to_string();
        exec_commands.commands = format!(
            "exec melies;{}",
            ifelse!(
                setting_as_bool(&settings["output"]["snd_fix"]),
                " snd_fix;",
                ""
            )
        );
    }

    {
        let start_record = vdm.create_action(ActionType::PlayCommands).props_mut();

        let clip_name_fallback = format!(
            "{}_{}-{}_{}",
            vdm_name, clip.start_tick, clip.end_tick, suffix
        );

        let mut clip_name = settings["output"]["clip_name_template"]
            .as_str()
            .unwrap_or(&clip_name_fallback)
            .replace("{demo_name}", &vdm_name)
            .replace("{start_tick}", &clip.start_tick.to_string())
            .replace("{end_tick}", &clip.end_tick.to_string())
            .replace("{suffix}", &suffix)
            .replace(
                "{recording_method}",
                &settings["output"]["method"].as_str().unwrap(),
            )
            .to_string();

        let mut bm_value = clip.bm_value.to_owned();

        bm_value = bm_value.replace("clip_start", "");
        bm_value = bm_value.replace("clip_end", "");

        if bm_value != "".to_string()
            && setting_as_bool(&settings["recording"]["auto_suffix"])
            && bm_value != "General".to_string()
        {
            clip_name = clip_name.replace(
                "{bookmarks}",
                &bm_value.replace("-spec", "").trim().replace(" ", "-"),
            );
        } else {
            clip_name = clip_name.replace("{bookmarks}", "");
        }

        let mut commands = "".to_string();

        match settings["output"]["method"].as_str().unwrap() {
            "h264" | "jpeg" => {
                commands = format!(
                    "host_framerate {}; startmovie {} {}; clear",
                    settings["output"]["framerate"],
                    clip_name,
                    settings["output"]["method"].as_str().unwrap()
                );
            }
            "tga" => {
                commands = format!(
                    "host_framerate {}; startmovie {}; clear",
                    settings["output"]["framerate"], clip_name
                );
            }
            "sparklyfx" => {
                if settings["output"]["folder"].as_str().unwrap().len() > 0 {
                    commands = format!(
                        "sf_recorder_start {}\\{}", // no need to change this as tf2 normalizes
                                                    // paths internally
                        settings["output"]["folder"].as_str().unwrap(),
                        clip_name
                    );
                } else {
                    commands = format!("sf_recorder_start; clear");
                }
            }
            "svr" => {
                commands = format!("startmovie {}.mkv", clip_name);
            }
            "svr.mp4" => {
                commands = format!("startmovie {}.mp4", clip_name);
            }
            "svr.mov" => {
                commands = format!("startmovie {}.mov", clip_name);
            }
            "lawena" => {
                commands = format!("startrecording");
            }
            _ => {}
        }

        let commands = check_spec(clip, commands);

        start_record.start_tick = Some(clip.start_tick);
        start_record.name = "Start Recording".to_string();
        start_record.commands = commands;
    }

    {
        let end_record = vdm.create_action(ActionType::PlayCommands).props_mut();
        let mut commands = String::new();

        match settings["output"]["method"].as_str().unwrap() {
            "h264" | "jpeg" | "tga" | "svr" | "svr.mp4" | "svr.mov" => {
                commands = format!(
                    "{}; endmovie; host_framerate 0;",
                    settings["recording"]["end_commands"].as_str().unwrap()
                );
            }
            "sparklyfx" => {
                commands = format!(
                    "{}; sf_recorder_stop;",
                    settings["recording"]["end_commands"].as_str().unwrap()
                );
            }
            "lawena" => {
                commands = format!(
                    "{}; stoprecording;",
                    settings["recording"]["end_commands"].as_str().unwrap()
                );
            }
            _ => {}
        }

        end_record.start_tick = Some(clip.end_tick);
        end_record.name = "Stop Recording".to_string();
        end_record.commands = commands;
    }
}

fn check_dir(files: Result<fs::ReadDir, std::io::Error>) -> Result<String, String> {
    let entries;

    match files {
        Ok(ent) => {
            entries = ent;
        }
        Err(_) => {
            return Err(
                "Could not find the _events.txt or KillStreaks.txt files.\nPlease check your settings to ensure the 'tf' folder is correctly linked.\nIf you do not have either file, please make one in the 'tf' or 'tf/demos' folder.".to_string()
            );
        }
    }

    for entry in entries {
        let dir: DirEntry;

        match entry {
            Ok(directory) => {
                dir = directory;
            }
            Err(err) => {
                return Err(err.to_string());
            }
        }

        let dir_str = dir.path().to_string_lossy().to_string();

        if dir_str.ends_with("\\_events.txt") {
            return Ok(dir_str);
        }

        if dir_str.ends_with("\\KillStreaks.txt") {
            return Ok(dir_str);
        }
    }

    Err(format!("File Not Found"))
}

fn find_dir(settings: &Value) -> Result<String, String> {
    let tf_folder = settings["tf_folder"].as_str();

    let files = match tf_folder {
        Some(tf_folder) => fs::read_dir(format!("{}\\demos", tf_folder)),
        None => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "tf_folder not set",
        )),
    };

    match check_dir(files) {
        Ok(res) => {
            return Ok(res);
        }
        Err(_) => {}
    }

    let tf_folder = settings["tf_folder"].as_str();
    let files = match tf_folder {
        Some(tf_folder) => fs::read_dir(format!("{}", tf_folder)),
        None => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "tf_folder not set",
        )),
    };

    match check_dir(files) {
        Ok(res) => {
            return Ok(res);
        }
        Err(_) => {}
    }

    match settings["tf_folder"].as_str() {
        Some(tf_folder) =>
            Err(
                format!("Could not find the _events.txt or KillStreaks.txt files.\nPlease check your settings to ensure the tf folder is correctly linked.\nIf you do not have either file, please make one in the 'tf' or 'tf/demos' folder. \n\ntf_folder setting: ({})", tf_folder)
            ),
        None => Err("tf_folder setting not set".to_string()),
    }
}

// Learn more about Tauri commands at https://tauri.app/v1/guides/features/command
#[command]
fn ryukbot() -> Value {
    let settings = load_settings();

    let dir;

    match find_dir(&settings) {
        Ok(directory) => {
            dir = directory;
        }
        Err(err) => {
            return json!({
                "code": 404,
                "err_text": err
            });
        }
    }

    let file_text = match fs::read_to_string(dir.clone()) {
        Ok(text) => text,
        Err(_) => {
            return json!({
                "code": 404,
                "err_text": "Failed to read _events.txt or KillStreaks.txt\nPlease check your settings to ensure the tf folder is correctly linked.".to_string()
            });
        }
    };

    let re = Regex::new("\\[(.*)\\] (.*) \\(\"(.*)\" at (\\d*)\\)").unwrap();

    let events = re.captures_iter(&file_text);

    let event_len = events.count();

    if event_len == 0 {
        return json!({
            "code": 410,
            "err_text": format!("_events.txt or KillStreaks.txt was found but had no valid events. Please add events before running again.\n\nFile Location: {}", dir)
        });
    }

    match write_cfg(&settings) {
        Ok(_) => {}
        Err(err) => {
            return json!({
                "code": 500,
                "err_text": err
            });
        }
    }

    let mut clips: Vec<Clip> = vec![];

    let mut event_count = 0;

    let events = re.captures_iter(&file_text);

    for event_capture in events {
        event_count = event_count + 1;

        let event = event::Event::new(event_capture).unwrap();

        if event.contains(&"mls_skip") {
            continue;
        }

        if event.contains(&"mls_rec_demo") {
            let demo = load_demo(&settings, &event.demo_name);

            println!("{}", demo);

            if !demo["loaded"].as_bool().unwrap() {
                continue;
            }

            if let Bookmark(val) = &event.value {
                let mut start_event = event.clone();
                let mut end_event = event.clone();

                let start_val = val.replace("mls_rec_demo", "clip_start");
                start_event.value = Bookmark(start_val);
                start_event.tick = settings["recording"]["start_delay"].as_i64().unwrap();

                end_event.value = Bookmark("clip_end".to_string());
                end_event.tick = demo["header"]["ticks"].as_i64().unwrap() - 99;

                let mut clip = Clip::new(start_event, &settings);

                if clip.can_include(&end_event, &settings) {
                    clip.include(end_event, &settings);
                }

                clips.push(clip);
                continue;
            }
        }

        let clip_len = clips.len();

        if clip_len == 0 {
            clips.push(Clip::new(event, &settings));
            continue;
        }

        if clips[clip_len - 1].can_include(&event, &settings) {
            clips.last_mut().unwrap().include(event, &settings);
            continue;
        }

        clips.push(Clip::new(event, &settings));
    }

    let mut current_demo: String = "".to_string();
    let mut vdm: VDM = VDM::new();
    let mut vdms = vec![];

    for clip in &clips {
        if current_demo == clip.demo_name {
            add_clip_to_vdm(&mut vdm, clip, &settings);
            continue;
        }

        if vdm.len() > 0 {
            vdms.push(vdm);
        }

        current_demo = clip.demo_name.clone();
        vdm = VDM::new();
        vdm.name = clip.demo_name.clone();
        start_vdm(&mut vdm, clip, &settings);
    }

    vdms.push(vdm);

    let first_demo = vdms[0].name.clone();

    let vdm_count = &vdms.len();

    for (i, vdm) in vdms.iter().enumerate() {
        let mut folder = format!("{}\\demos", &settings["tf_folder"].as_str().unwrap());

        if settings["absolute_file_paths"].as_bool().unwrap() {
            folder = format!("{}", &settings["tf_folder"].as_str().unwrap());
        }

        let file_location = format!("{}\\{}.vdm", folder, &vdm.name);

        let path = Path::new(&file_location);

        if !path.parent().unwrap().exists() {
            fs::create_dir_all(path).unwrap();
        }

        if settings["safe_mode"].as_i64().is_some() {
            if setting_as_bool(&settings["safe_mode"]) {
                let file_path = Path::new(&file_location);
                if file_path.exists() {
                    continue;
                }
            }
        }

        let vdm = end_vdm(
            &mut vdm.clone(),
            &settings,
            ifelse!(
                vdms.len() > i + 1,
                String::from(&vdms[i + 1].name),
                String::new()
            ),
        );

        vdm.export(&file_location);
    }

    let mut backup_location = "".to_string();

    if settings["save_backups"].as_bool().unwrap() {
        let saved = save_backup(&settings);

        backup_location = saved["output_path"].as_str().unwrap().to_owned();
    }

    if settings["clear_events"].as_bool().unwrap() {
        clear_events(settings);
    }

    json!({
        "clips": clips.len(),
        "events": event_count,
        "vdms": vdm_count,
        "code": 200,
        "backup_location": backup_location,
        "first_demo": first_demo
    })
}

#[command]
fn load_settings() -> Value {
    settings::load_settings()
}

#[command]
fn save_settings(new_settings: String) -> Value {
    settings::save_settings(new_settings)
}

#[command]
fn load_events() -> Value {
    let settings = load_settings();

    let dir;

    match find_dir(&settings) {
        Ok(directory) => {
            dir = directory;
        }
        Err(err) => {
            return json!({
                "code": 404,
                "err_text": err
            });
        }
    }

    let file_text = match fs::read_to_string(dir) {
        Ok(text) => text,
        Err(err) => {
            return json!({
                "code": 400,
                "err_text": err.to_string()
            });
        }
    };

    let re = Regex::new("\\[(.*)\\] (.*) \\(\"(.*)\" at (\\d*)\\)").unwrap();

    let events_regex = re.captures_iter(&file_text);

    let mut events = vec![];

    for event_capture in events_regex {
        let event = event::Event::new(event_capture).unwrap();

        events.push(event);
    }

    json!({
        "code": 200,
        "events": events
    })
}

#[command]
fn save_events(new_events: Value) -> Value {
    let mut events: Vec<Event> = vec![];
    let mut new_events_text = String::new();

    for demo in new_events.as_array().unwrap() {
        extend!(new_events_text, "{}\n", ">");

        for event in demo.as_array().unwrap() {
            let re = Regex::new("\\[(.*)\\] (.*) \\(\"(.*)\" at (\\d*)\\)").unwrap();

            if event["event"].as_str().is_none() {
                continue;
            }

            let events_regex = match re.captures(event["event"].as_str().unwrap()) {
                Some(val) => val,
                None => {
                    continue;
                }
            };

            let original_event = event::Event::new(events_regex).unwrap();

            if event["demo_name"].as_str().unwrap() != original_event.demo_name {
                let built_event = build_event_from_json(event);
                extend!(new_events_text, "{}\n", built_event.event);
                events.push(built_event);
                continue;
            }

            if event["tick"].as_i64().unwrap() != original_event.tick {
                let built_event = build_event_from_json(event);
                extend!(new_events_text, "{}\n", built_event.event);
                events.push(built_event);
                continue;
            }

            match &original_event.value {
                Bookmark(bm) => {
                    if event["isKillstreak"].as_bool().unwrap() {
                        let built_event = build_event_from_json(event);
                        extend!(new_events_text, "{}\n", built_event.event);
                        events.push(built_event);
                        continue;
                    }

                    if bm.to_owned() != event["value"]["Bookmark"].as_str().unwrap() {
                        let built_event = build_event_from_json(event);
                        extend!(new_events_text, "{}\n", built_event.event);
                        events.push(built_event);
                        continue;
                    }
                }
                Killstreak(ks) => {
                    if !event["isKillstreak"].as_bool().unwrap() {
                        let built_event = build_event_from_json(event);
                        extend!(new_events_text, "{}\n", built_event.event);
                        events.push(built_event);
                        continue;
                    }

                    if ks.to_owned() != event["value"]["Killstreak"].as_i64().unwrap() {
                        let built_event = build_event_from_json(event);
                        extend!(new_events_text, "{}\n", built_event.event);
                        events.push(built_event);
                        continue;
                    }
                }
            }

            extend!(new_events_text, "{}\n", original_event.event);
            events.push(original_event);
        }
    }

    let settings = load_settings();

    let dir;

    match find_dir(&settings) {
        Ok(directory) => {
            dir = directory;
        }
        Err(err) => {
            return json!({
                "code": 404,
                "err_text": err
            });
        }
    }

    fs::write(dir, new_events_text).unwrap();

    return json!({
        "code": 200,
        "events": events
    });
}

fn build_event_from_json(event_json: &Value) -> Event {
    let sys_time: DateTime<Local> = Local::now();

    match event_json["isKillstreak"].as_bool().unwrap() {
        true => {
            return Event {
                event: format!(
                    "[{}] Killstreak {} (\"{}\" at {})",
                    sys_time
                        .format("%Y/%m/%d %H:%M")
                        .to_string()
                        .replace("\"", ""),
                    event_json["value"]["Killstreak"],
                    event_json["demo_name"].as_str().unwrap(),
                    event_json["tick"].as_i64().unwrap()
                ),
                demo_name: event_json["demo_name"].as_str().unwrap().to_string(),
                tick: event_json["tick"].as_i64().unwrap(),
                value: Killstreak(event_json["value"]["Killstreak"].as_i64().unwrap()),
            };
        }
        false => {
            if event_json["value"]["Bookmark"] == "General" {
                return Event {
                    event: format!(
                        "[{}] Bookmark {} (\"{}\" at {})",
                        sys_time.format("%Y/%m/%d %H:%M").to_string(),
                        event_json["value"]["Bookmark"].as_str().unwrap(),
                        event_json["demo_name"].as_str().unwrap(),
                        event_json["tick"].as_i64().unwrap()
                    ),
                    demo_name: event_json["demo_name"].as_str().unwrap().to_string(),
                    tick: event_json["tick"].as_i64().unwrap(),
                    value: Bookmark(
                        event_json["value"]["Bookmark"]
                            .as_str()
                            .unwrap()
                            .to_string(),
                    ),
                };
            }

            return Event {
                event: format!(
                    "[{}] {} (\"{}\" at {})",
                    sys_time.format("%Y/%m/%d %H:%M").to_string(),
                    event_json["value"]["Bookmark"].as_str().unwrap(),
                    event_json["demo_name"].as_str().unwrap(),
                    event_json["tick"].as_i64().unwrap()
                ),
                demo_name: event_json["demo_name"].as_str().unwrap().to_string(),
                tick: event_json["tick"].as_i64().unwrap(),
                value: Bookmark(
                    event_json["value"]["Bookmark"]
                        .as_str()
                        .unwrap()
                        .to_string(),
                ),
            };
        }
    }
}

fn clear_events(settings: Value) -> Value {
    let dir;

    match find_dir(&settings) {
        Ok(directory) => {
            dir = directory;
        }
        Err(err) => {
            return json!({
                "code": 404,
                "err_text": err
            });
        }
    }

    File::create(dir).unwrap();

    json!({
        "code": 200,
        "events": []
    })
}

fn save_backup(settings: &Value) -> Value {
    let dir;

    match find_dir(&settings) {
        Ok(directory) => {
            dir = directory;
        }
        Err(err) => {
            return json!({
                "code": 404,
                "err_text": err
            });
        }
    }

    let sys_time: DateTime<Local> = Local::now();
    let date = sys_time
        .format("%Y-%m-%d_%H-%M-%S")
        .to_string()
        .replace("\"", "");

    let user_profile = env::var("USERPROFILE");

    let output_folder = match user_profile {
        Ok(profile) => BACKUPS_DIR.join(profile),
        Err(_) => BACKUPS_DIR.to_owned(),
    };

    let output_path = output_folder.join(date);

    fs::create_dir_all(output_folder).unwrap();

    fs::copy(dir, &output_path).unwrap();

    json!({
        "code": 200,
        "output_path": output_path
    })
}

#[command]
fn parse_log(url: Value) -> Value {
    parse(url)
}

const FIX_TF_FOLDER: &str =
    "Can't find 'tf' folder. Please fix the \"'tf' Folder\" setting in settings.";

#[command]
fn load_vdms() -> Result<Value, String> {
    let settings = load_settings();
    if validate_demos_folder(&settings) {
        return Ok(scan_for_vdms(settings));
    }

    Err(String::from(FIX_TF_FOLDER))
}

#[command]
fn load_demos() -> Result<Value, String> {
    let settings = load_settings();
    if validate_demos_folder(&settings) {
        return Ok(scan_for_demos(settings));
    }

    Err(String::from(FIX_TF_FOLDER))
}

pub(crate) fn validate_backups_folder() -> bool {
    let user_profile = env::var("USERPROFILE");

    let backups_folder = match user_profile {
        Ok(profile) => BACKUPS_DIR.join(profile),
        Err(_) => BACKUPS_DIR.to_owned(),
    };

    match fs::read_dir(backups_folder) {
        Ok(_) => {
            return true;
        }
        Err(_) => {
            return false;
        }
    }
}

pub(crate) fn scan_for_backups() -> Value {
    let mut events: Vec<Value> = vec![];

    let user_profile = env::var("USERPROFILE");

    let backups_folder = match user_profile {
        Ok(profile) => BACKUPS_DIR.join(profile),
        Err(_) => BACKUPS_DIR.to_owned(),
    };

    for entry in fs::read_dir(backups_folder).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();

        if path.is_file()
            && path
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .ends_with(".txt")
        {
            events.push(json!({
                "file_name": path.file_name().unwrap().to_str().unwrap().to_string(),
                "created": entry.metadata().unwrap().created().unwrap(),
            }));
        }
    }

    json!(events)
}

#[command]
fn load_backups() -> Result<Value, String> {
    if validate_backups_folder() {
        return Ok(scan_for_backups());
    }

    Err(String::from(
        "Can't find backups folder. You may not have backups yet.",
    ))
}

#[command]
fn reload_backup(file_name: Value) -> Result<Value, String> {
    let user_profile = env::var("USERPROFILE");

    let backups_folder = match user_profile {
        Ok(profile) => BACKUPS_DIR.join(profile),
        Err(_) => BACKUPS_DIR.to_owned(),
    };

    let file_path = backups_folder.join(file_name.as_str().unwrap());
    let backup_file = Path::new(&file_path);

    let settings = load_settings();

    let dir;

    match find_dir(&settings) {
        Ok(directory) => {
            dir = directory;
        }
        Err(_) => {
            return Err(String::from("Failed to reload backup."));
        }
    }

    println!("Reloading backup: {}", file_path.display());

    if backup_file.exists() {
        let copy = fs::copy(file_path, dir);

        if copy.is_ok() {
            return Ok(json!("Successfully loaded backup."));
        }

        return Err(String::from("Failed to reload backup."));
    }

    Err(String::from("Failed to find backup file."))
}

#[command]
fn parse_demo(path: String) -> Value {
    scan_demo(load_settings(), path)
}

#[command]
fn load_vdm(name: Value) -> Value {
    let settings = load_settings();

    let dir = format!(
        "{}{}",
        settings["tf_folder"].as_str().unwrap(),
        name.as_str().unwrap()
    );

    let vdm = VDM::open(&dir).unwrap();

    vdm_to_json(vdm)
}

#[command]
fn save_vdm(name: Value, vdm: Value) -> Value {
    let settings = load_settings();

    let dir = format!(
        "{}{}",
        settings["tf_folder"].as_str().unwrap(),
        name.as_str().unwrap()
    );

    let vdm = json_to_vdm(vdm);

    vdm.export(&dir);

    json!({
        "success": true
    })
}

#[command]
fn open_addons_folder() {
    addons::open_addons_folder();
}

#[command]
fn delete_demo(file_name: Value) {
    let settings = load_settings();

    let file_path = format!(
        "{}{}",
        settings["tf_folder"].as_str().unwrap(),
        file_name.as_str().unwrap()
    );
    let vdm_file_path = format!(
        "{}{}",
        settings["tf_folder"].as_str().unwrap(),
        file_name.as_str().unwrap().replace(".dem", ".vdm")
    );

    let path = Path::new(&file_path);
    let vdm_path = Path::new(&vdm_file_path);

    if path.exists() {
        trash::delete(path).unwrap();
    }

    if vdm_path.exists() {
        trash::delete(vdm_path).unwrap();
    }
}

#[command]
fn delete_vdm(file_name: Value) {
    let settings = load_settings();

    let file_path = format!(
        "{}{}",
        settings["tf_folder"].as_str().unwrap(),
        file_name.as_str().unwrap().replace(".dem", ".vdm")
    );

    let path = Path::new(&file_path);

    if path.exists() {
        trash::delete(path).unwrap();
    }
}

#[command]
fn create_vdm(file_name: Value) {
    let settings = load_settings();

    let file_path = format!(
        "{}{}",
        settings["tf_folder"].as_str().unwrap(),
        file_name.as_str().unwrap().replace(".dem", ".vdm")
    );

    let path = Path::new(&file_path);

    if !path.exists() {
        let vdm = VDM::new();
        vdm.export(&file_path);
    }
}

#[command]
fn load_theme() -> Value {
    let user_profile = env::var("USERPROFILE");

    let settings_path = match user_profile {
        Ok(profile) => CONF_DIR.join(profile).join("theme.json"),
        Err(_) => CONF_DIR.join("theme.json"),
    };

    if Path::new(&settings_path).exists() {
        let file = fs::read_to_string(settings_path).unwrap();
        let mut theme: Value = serde_json::from_str(&file).unwrap();

        theme["has_theme"] = json!(true);

        return theme;
    }

    return json!({
        "has_theme": false
    });
}

#[command]
fn open_themes_folder() {
    let user_profile = env::var("USERPROFILE");

    let addons_path = match user_profile {
        Ok(profile) => &*CONF_DIR.join(profile),
        Err(_) => &*CONF_DIR,
    };

    fs::create_dir_all(&addons_path).unwrap();

    impl_open_file(&addons_path);
}

#[command]
fn open_install_folder(install: &str) {
    fs::create_dir_all(install).unwrap();

    impl_open_file(install);
}

#[command]
fn rename_file(old_path: &str, new_path: &str) {
    let path = Path::new(old_path);
    let new_path = Path::new(new_path);

    let sanitized_path = sanitize_name(new_path);

    fs::rename(path, sanitized_path).unwrap();
}

#[command]
fn cleanup_rename(demo_map: Value) {
    let events_obj = load_events();

    let events = match events_obj["events"].as_array() {
        Some(val) => val.to_owned(),
        None => {
            return;
        }
    };

    save_events(cleanup_renamed_events(demo_map.clone(), events));
    let settings = load_settings();

    cleanup_renamed_vdms(
        demo_map,
        scan_for_vdms(load_settings()),
        settings["tf_folder"].as_str().unwrap(),
    );
}

fn sanitize_name(path: &Path) -> PathBuf {
    let mut result = path.to_owned();

    let file_name = path.file_name().unwrap().to_str().unwrap();
    let sanitized = sanitize(file_name);

    result.set_file_name(sanitized);

    if let Some(ext) = path.extension() {
        result.set_extension(ext);
    }

    result
}

#[command]
fn is_steam_running() -> bool {
    use sysinfo::System;

    let s = System::new_all();

    let mut process_found = false;

    for _process in s.processes_by_name("steam".as_ref()) {
        process_found = true;
        break;
    }

    process_found
}

#[command]
fn launch_tf2(demo_name: &str, install: &str, tab: &str) -> Value {
    tf2::run_tf2(demo_name, &load_settings(), install, tab, true)
}

#[command]
fn batch_record(demo_name: &str, install: &str, tab: &str, first_run: bool) -> Value {
    tf2::batch_record(demo_name, &load_settings(), install, tab, first_run)
}

#[tauri::command]
fn load_files(folder: &str) -> Value {
    let path = std::path::Path::new(folder);
    if !path.exists() {
        return json!({});
    }

    let entries = std::fs::read_dir(path).unwrap();

    let mut videos: Vec<Value> = vec![];

    for entry in entries {
        let path = entry.unwrap().path();
        if !path.is_dir() {
            continue;
        }

        let has_layers = std::fs::read_dir(path.join("take0000"));

        let layers = match has_layers {
            Ok(layers) => layers,
            Err(_) => continue,
        };

        let mut video_layers = json!({});

        for layer in layers {
            let path = layer.unwrap().path();
            if path.is_dir() {
                continue;
            }

            let file_name = path.file_name().unwrap().to_str().unwrap();

            if !matches!(file_name.rsplit('.').next(), Some("mp4" | "avi" | "mkv")) {
                continue;
            }

            video_layers[file_name
                .replace(".mp4", "")
                .replace(".avi", "")
                .replace(".mkv", "")] = json!(path.to_str().unwrap().to_string());
        }

        let video: Value = json!({
            "name": path.file_name().unwrap().to_str().unwrap(),
            "path": path.to_str().unwrap(),
            "layers": video_layers
        });

        videos.push(video);
    }

    json!(videos)
}

fn impl_open_file(path: impl AsRef<OsStr>) {
    opener::open(path).unwrap();
}

fn impl_delete_file(path: impl AsRef<Path>) {
    trash::delete(path).unwrap();
}

#[tauri::command]
fn open_file(path: &str) {
    impl_open_file(path)
}

#[tauri::command]
fn delete_file(path: &str) {
    impl_delete_file(path)
}

#[tauri::command]
fn build_install(folder_name: &str) -> Value {
    tf2::build_new_install(folder_name, &load_settings())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            ryukbot,
            load_settings,
            save_settings,
            load_events,
            save_events,
            parse_log,
            load_vdm,
            load_vdms,
            save_vdm,
            load_demos,
            load_backups,
            reload_backup,
            parse_demo,
            delete_demo,
            delete_vdm,
            create_vdm,
            load_theme,
            open_themes_folder,
            open_addons_folder,
            open_install_folder,
            rename_file,
            cleanup_rename,
            is_steam_running,
            launch_tf2,
            batch_record,
            load_files,
            open_file,
            delete_file,
            build_install
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

static CONF_DIR: LazyLock<PathBuf> = LazyLock::new(|| {
    if cfg!(windows) {
        // Microsoft doesn't know what a standard is
        home_dir().map(|d| d.join("Documents").join("Melies"))
    } else {
        // MacOS, Linux, *BSD, etc.
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .map(|d| d.join("Melies"))
            .or_else(|| home_dir().map(|d| d.join(".config").join("Melies")))
    }
    .unwrap_or_else(|| current_dir().unwrap())
});

static BACKUPS_DIR: LazyLock<PathBuf> = LazyLock::new(|| CONF_DIR.join("backups"));
