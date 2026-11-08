use crate::{
    ffi::{Api, CommandReply, Event, Node, Value},
    pixels,
};
use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value as Json};
use std::{
    ffi::{c_void, CString},
    path::Path,
    time::Instant,
};

pub struct Session<'a> {
    api: &'a Api,
    handle: *mut c_void,
    request_id: u64,
    loaded: bool,
    started: bool,
    restarted: bool,
}

impl<'a> Session<'a> {
    pub fn new(api: &'a Api) -> Result<Self> {
        // SAFETY: loaded functions match client.h; the DLL outlives this session.
        let handle = unsafe { (api.create)() };
        ensure!(!handle.is_null(), "mpv_create failed");
        let session = Self {
            api,
            handle,
            request_id: 0,
            loaded: false,
            started: false,
            restarted: false,
        };
        for (name, value) in [
            ("config", "no"),
            ("load-scripts", "no"),
            ("ytdl", "no"),
            ("access-references", "no"),
            ("terminal", "no"),
            ("msg-level", "all=no"),
            ("vo", "null"),
            ("hwdec", "no"),
            ("pause", "yes"),
            ("keep-open", "always"),
            ("idle", "yes"),
            ("audio", "no"),
            ("sub", "no"),
            ("audio-file-auto", "no"),
            ("sub-auto", "no"),
            ("cover-art-auto", "no"),
            ("osd-level", "0"),
            ("input-default-bindings", "no"),
            ("input-vo-keyboard", "no"),
            ("cache", "no"),
            ("demuxer-max-bytes", "8MiB"),
            ("demuxer-max-back-bytes", "1MiB"),
        ] {
            let key = CString::new(name)?;
            let value = CString::new(value)?;
            api.check(
                unsafe { (api.set_option)(handle, key.as_ptr(), value.as_ptr()) },
                name,
            )?;
        }
        api.check(unsafe { (api.initialize)(handle) }, "mpv_initialize")?;
        Ok(session)
    }

    pub fn property(&self, name: &str) -> Result<Json> {
        let name = CString::new(name)?;
        let mut node = Node {
            value: Value { integer: 0 },
            format: 0,
        };
        self.api.check(
            unsafe {
                (self.api.get_property)(
                    self.handle,
                    name.as_ptr(),
                    6,
                    (&mut node as *mut Node).cast(),
                )
            },
            "get_property",
        )?;
        let result = node.json();
        // SAFETY: successful MPV_FORMAT_NODE property reads transfer contents to us.
        unsafe { (self.api.free_node)(&mut node) };
        result
    }

    pub fn inspect(&self) -> Json {
        json!({
            "mpv_version": self.property("mpv-version").ok(),
            "ffmpeg_version": self.property("ffmpeg-version").ok(),
            "client_api": unsafe { (self.api.client_version)() },
            "decoders": self.property("decoder-list").ok(),
            "capture_backend": "screenshot-raw/video; vo=null; hwdec=no",
        })
    }

    fn next_event(&mut self, deadline: Instant, allow_end: bool) -> Result<&Event> {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| anyhow::anyhow!("Timed out waiting for libmpv"))?;
        // Event data remains valid until the next wait on this handle or destruction.
        let event = unsafe { (self.api.wait)(self.handle, remaining.as_secs_f64().min(0.05)) };
        let event = unsafe { event.as_ref() }.context("Null libmpv event")?;
        match event.id {
            1 => bail!("libmpv shut down unexpectedly"),
            6 => self.started = true,
            7 => {
                self.loaded = false;
                self.started = false;
                if !allow_end {
                    bail!("Media ended before capture (unreadable media or no video stream)");
                }
            }
            8 => self.loaded = true,
            21 => self.restarted = true,
            24 => bail!("libmpv event queue overflow"),
            _ => {}
        }
        Ok(event)
    }

    pub(crate) fn request(
        &mut self,
        args: &[&str],
        deadline: Instant,
        max_side: Option<u32>,
        allow_end: bool,
    ) -> Result<Option<pixels::Screenshot>> {
        let strings = args
            .iter()
            .map(|arg| CString::new(*arg))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut pointers = strings.iter().map(|s| s.as_ptr()).collect::<Vec<_>>();
        pointers.push(std::ptr::null());
        self.request_id += 1;
        let id = self.request_id;
        self.api.check(
            unsafe { (self.api.command)(self.handle, id, pointers.as_ptr()) },
            args[0],
        )?;
        loop {
            let api = self.api;
            let event = self.next_event(deadline, allow_end)?;
            if event.id != 5 || event.userdata != id {
                continue;
            }
            api.check(event.error, args[0])?;
            if let Some(max_side) = max_side {
                let reply = unsafe { event.data.cast::<CommandReply>().as_ref() }
                    .context("Missing screenshot command result")?;
                // Copy pixels before the next mpv_wait_event invalidates the result.
                return Ok(Some(pixels::screenshot(&reply.result, max_side)?));
            }
            return Ok(None);
        }
    }

    pub(crate) fn stop(&mut self, deadline: Instant) -> Result<()> {
        // Finish the previous file before resetting event state (important for reuse).
        self.request(&["stop"], deadline, None, true)?;
        // The command reply can precede END_FILE. A nonblocking queue drain alone
        // races with teardown and misattributes the old END_FILE to the new file.
        // START_FILE also covers a probe aborted before FILE_LOADED. A queued
        // load cancelled before START_FILE has no END_FILE to wait for.
        while self.started || self.loaded {
            self.next_event(deadline, true)?;
        }
        loop {
            ensure!(
                Instant::now() < deadline,
                "Timed out draining old libmpv events"
            );
            let event = unsafe { (self.api.wait)(self.handle, 0.0).as_ref() }
                .context("Null libmpv event")?;
            if event.id == 0 {
                break;
            }
        }
        self.loaded = false;
        self.started = false;
        self.restarted = false;
        Ok(())
    }

    pub(crate) fn load(&mut self, path: &Path, deadline: Instant) -> Result<()> {
        self.load_video(path, None, "auto", deadline)
    }

    pub(crate) fn load_tracks(&mut self, path: &Path, deadline: Instant) -> Result<()> {
        // Inspect attachment metadata without initializing mpv's main video decoder.
        self.load_file(
            path,
            "demuxer=lavf,vid=no,track-auto-selection=no",
            deadline,
        )
    }

    pub(crate) fn load_video(
        &mut self,
        path: &Path,
        demuxer: Option<&str>,
        video: &str,
        deadline: Instant,
    ) -> Result<()> {
        // Never carry a selected cover or forced demuxer into the next file.
        let options = format!(
            "demuxer={},vid={video},track-auto-selection=yes",
            demuxer.unwrap_or_default()
        );
        self.load_file(path, &options, deadline)?;
        while !self.restarted {
            self.next_event(deadline, false)?;
        }
        let vid = self.property("vid")?;
        ensure!(
            vid.as_i64().is_some_and(|id| id > 0),
            "No selected video stream"
        );
        Ok(())
    }

    fn load_file(&mut self, path: &Path, options: &str, deadline: Instant) -> Result<()> {
        let path = path.to_str().context("Input path is not valid Unicode")?;
        self.request(
            &["loadfile", path, "replace", "-1", options],
            deadline,
            None,
            false,
        )?;
        while !self.loaded {
            self.next_event(deadline, false)?;
        }
        Ok(())
    }

    pub(crate) fn seek(&mut self, target: f64, exact: bool, deadline: Instant) -> Result<()> {
        self.restarted = false;
        self.request(
            &[
                "seek",
                &target.to_string(),
                if exact {
                    "absolute+exact"
                } else {
                    "absolute+keyframes"
                },
            ],
            deadline,
            None,
            false,
        )?;
        while !self.restarted {
            self.next_event(deadline, false)?;
        }
        Ok(())
    }

    pub(crate) fn scaling(&mut self, max_side: Option<u32>, deadline: Instant) -> Result<()> {
        let filter = max_side.map(|side| format!(
            "lavfi=graph=[scale={side}:{side}:force_original_aspect_ratio=decrease:reset_sar=1:flags=bilinear]:o=[threads=1]"
        )).unwrap_or_default();
        self.request(&["vf", "set", &filter], deadline, None, false)?;
        Ok(())
    }
}

impl Drop for Session<'_> {
    fn drop(&mut self) {
        // No event/pixel pointers escape Session. Teardown runs on its owning thread.
        unsafe { (self.api.destroy)(self.handle) };
    }
}
