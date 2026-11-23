//! Microphone capture on the default input device, reopened when the default changes (headset plugged in,
//! Bluetooth switched). Talks to Core Audio directly, like the tap, so the page never asks WKWebView for the
//! microphone. Needs `NSMicrophoneUsageDescription` in Info.plist.
use super::frame::Framer;
use super::tap::{check, check_format, mono_of, Awake, IoBlock, ListenerBlock, NON_INTERLEAVED};
use block2::{RcBlock, StackBlock};
use objc2::{
    class, msg_send,
    rc::Retained,
    runtime::{AnyClass, Bool},
};
use objc2_core_audio::*;
use objc2_core_audio_types::{AudioBufferList, AudioStreamBasicDescription, AudioTimeStamp};
use objc2_foundation::NSString;
use std::{
    ffi::c_void,
    ptr::NonNull,
    sync::{
        atomic::{AtomicU32, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::JoinHandle,
};

pub type OnFrame = Arc<dyn Fn(&[i16]) + Send + Sync>;

#[link(name = "AVFoundation", kind = "framework")]
extern "C" {}

const SYSTEM: AudioObjectID = kAudioObjectSystemObject as u32;

const DEFAULT_INPUT: AudioObjectPropertyAddress = AudioObjectPropertyAddress {
    mSelector: kAudioHardwarePropertyDefaultInputDevice,
    mScope: kAudioObjectPropertyScopeGlobal,
    mElement: kAudioObjectPropertyElementMain,
};

const INPUT_FORMAT: AudioObjectPropertyAddress = AudioObjectPropertyAddress {
    mSelector: kAudioDevicePropertyStreamFormat,
    mScope: kAudioObjectPropertyScopeInput,
    mElement: kAudioObjectPropertyElementMain,
};

unsafe fn get<T: Default>(
    id: AudioObjectID,
    addr: &AudioObjectPropertyAddress,
) -> Result<T, String> {
    let mut out = T::default();
    let mut size = size_of::<T>() as u32;
    check(
        "read an audio property",
        AudioObjectGetPropertyData(
            id,
            NonNull::from(addr),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::from(&mut out).cast::<c_void>(),
        ),
    )?;
    Ok(out)
}

unsafe fn input_format(device: AudioObjectID) -> Result<AudioStreamBasicDescription, String> {
    let mut f = std::mem::zeroed::<AudioStreamBasicDescription>();
    let mut size = size_of::<AudioStreamBasicDescription>() as u32;
    check(
        "read the microphone format",
        AudioObjectGetPropertyData(
            device,
            NonNull::from(&INPUT_FORMAT),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::from(&mut f).cast::<c_void>(),
        ),
    )?;
    Ok(f)
}

/// One open default input device. Dropping it stops the callback.
struct Stream {
    device: AudioObjectID,
    io_proc: AudioDeviceIOProcID,
    _block: RcBlock<IoBlock>,
    // Re-reads the sample rate when it changes (a Bluetooth headset switching profile).
    listener: RcBlock<ListenerBlock>,
}

// Only touched from `open` and `drop`, never from the audio thread.
unsafe impl Send for Stream {}

impl Stream {
    fn open(on_frame: &OnFrame) -> Result<Stream, String> {
        unsafe {
            let device: AudioObjectID = get(SYSTEM, &DEFAULT_INPUT)?;
            if device == 0 {
                return Err("no microphone found".into());
            }
            let format = input_format(device)?;
            check_format(&format).map_err(|_| "the microphone is not 32-bit float".to_string())?;
            if format.mSampleRate < 1.0 || format.mChannelsPerFrame == 0 {
                return Err("the microphone is not ready".into());
            }
            let channels = format.mChannelsPerFrame as usize;
            let planar = format.mFormatFlags & NON_INTERLEAVED != 0;

            let rate = Arc::new(AtomicU32::new(format.mSampleRate as u32));
            let listener_rate = rate.clone();
            let listener = RcBlock::new(move |_: u32, _: NonNull<AudioObjectPropertyAddress>| {
                match input_format(device) {
                    Ok(f) if f.mSampleRate >= 1.0 => {
                        listener_rate.store(f.mSampleRate as u32, Ordering::Relaxed)
                    }
                    _ => {}
                }
            });
            check(
                "watch the microphone format",
                AudioObjectAddPropertyListenerBlock(
                    device,
                    NonNull::from(&INPUT_FORMAT),
                    None,
                    RcBlock::as_ptr(&listener),
                ),
            )?;

            let on_frame = on_frame.clone();
            let framer = Mutex::new(Framer::new(rate.load(Ordering::Relaxed)));
            let block = RcBlock::new(
                move |_: NonNull<AudioTimeStamp>,
                      input: NonNull<AudioBufferList>,
                      _: NonNull<AudioTimeStamp>,
                      _: NonNull<AudioBufferList>,
                      _: NonNull<AudioTimeStamp>| {
                    let mono = mono_of(input.as_ref(), planar, channels);
                    let mut framer = framer.lock().unwrap();
                    let now = rate.load(Ordering::Relaxed);
                    if now != framer.rate() {
                        *framer = Framer::new(now);
                    }
                    framer.push(&mono, &*on_frame);
                },
            );
            let mut s = Stream {
                device,
                io_proc: None,
                _block: block,
                listener,
            };
            check(
                "register the microphone callback",
                AudioDeviceCreateIOProcIDWithBlock(
                    NonNull::from(&mut s.io_proc),
                    device,
                    None,
                    RcBlock::as_ptr(&s._block),
                ),
            )?;
            check("start the microphone", AudioDeviceStart(device, s.io_proc))?;
            Ok(s)
        }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        unsafe {
            if self.io_proc.is_some() {
                AudioDeviceStop(self.device, self.io_proc);
                AudioDeviceDestroyIOProcID(self.device, self.io_proc);
            }
            AudioObjectRemovePropertyListenerBlock(
                self.device,
                NonNull::from(&INPUT_FORMAT),
                None,
                RcBlock::as_ptr(&self.listener),
            );
        }
    }
}

/// The microphone as a running capture that follows the default input device.
pub struct Mic {
    // `true` = the default input changed, `false` = shut down. The reopen runs on its own thread because
    // tearing a device down inside a Core Audio listener can deadlock.
    tx: mpsc::Sender<bool>,
    worker: Option<JoinHandle<()>>,
    listener: RcBlock<ListenerBlock>,
    _awake: Awake,
}

unsafe impl Send for Mic {}

impl Mic {
    /// `on_frame` gets 1600 mono i16 samples at 16 kHz, on an audio thread. Fails if there is no input device
    /// now; a swap is retried for about a second, then the capture stays silent until the next change.
    pub fn start(on_frame: OnFrame) -> Result<Mic, String> {
        // A denied microphone delivers silence, not an error, so refuse here for a page that skipped the check.
        can_record()?;
        let stream = Mutex::new(Some(Stream::open(&on_frame)?));
        let (tx, rx) = mpsc::channel::<bool>();
        let worker = std::thread::spawn(move || {
            while rx.recv() == Ok(true) {
                let mut s = stream.lock().unwrap();
                *s = None; // release the old device before opening the new one
                           // A device mid-switch (Bluetooth profile change) often fails its first open.
                for _ in 0..5 {
                    if let Ok(new) = Stream::open(&on_frame) {
                        *s = Some(new);
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
            }
        });
        let notify = Mutex::new(tx.clone());
        let listener = RcBlock::new(move |_: u32, _: NonNull<AudioObjectPropertyAddress>| {
            let _ = notify.lock().unwrap().send(true);
        });
        let status = unsafe {
            AudioObjectAddPropertyListenerBlock(
                SYSTEM,
                NonNull::from(&DEFAULT_INPUT),
                None,
                RcBlock::as_ptr(&listener),
            )
        };
        let mic = Mic {
            tx,
            worker: Some(worker),
            listener,
            _awake: Awake::new(),
        };
        check("watch the default microphone", status)?; // on error `mic` drops and stops the worker
        Ok(mic)
    }
}

impl Drop for Mic {
    fn drop(&mut self) {
        unsafe {
            AudioObjectRemovePropertyListenerBlock(
                SYSTEM,
                NonNull::from(&DEFAULT_INPUT),
                None,
                RcBlock::as_ptr(&self.listener),
            );
        }
        let _ = self.tx.send(false);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

/// Microphone permission: `Ok` when granted. The first call asks the member and waits for the answer.
pub fn can_record() -> Result<(), String> {
    unsafe {
        let cls: &AnyClass = class!(AVCaptureDevice);
        let media: Retained<NSString> = NSString::from_str("soun"); // AVMediaTypeAudio
        let mut status: isize = msg_send![cls, authorizationStatusForMediaType: &*media];
        if status == 0 {
            let (tx, rx) = mpsc::channel();
            let done = StackBlock::new(move |ok: Bool| {
                let _ = tx.send(ok.as_bool());
            });
            let _: () = msg_send![cls, requestAccessForMediaType: &*media, completionHandler: &*done.copy()];
            status = if rx.recv() == Ok(true) { 3 } else { 2 };
        }
        // 0 not asked, 1 restricted, 2 denied, 3 authorized
        if status == 3 {
            Ok(())
        } else {
            Err("microphone".into())
        }
    }
}
