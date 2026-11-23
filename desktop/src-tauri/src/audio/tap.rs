//! Global Core Audio process tap (macOS 14.2+): everything the Mac plays, mixed to stereo, with no Screen
//! Recording permission. The tap is not bound to an output device, so switching to AirPods keeps it running.
//! Needs `NSAudioCaptureUsageDescription` in Info.plist; macOS asks the member the first time it starts.
use super::frame::{downmix, Framer};
use block2::RcBlock;
use objc2::{rc::Retained, runtime::AnyObject, runtime::ProtocolObject, AllocAnyThread};
use objc2_core_audio::*;
use objc2_core_audio_types::{AudioBufferList, AudioStreamBasicDescription, AudioTimeStamp};
use objc2_core_foundation::CFDictionary;
use objc2_foundation::*;
use std::{
    ffi::c_void,
    ptr::NonNull,
    slice,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc, Mutex,
    },
};

pub(super) type IoBlock = dyn Fn(
    NonNull<AudioTimeStamp>,
    NonNull<AudioBufferList>,
    NonNull<AudioTimeStamp>,
    NonNull<AudioBufferList>,
    NonNull<AudioTimeStamp>,
);
pub(super) type ListenerBlock = dyn Fn(u32, NonNull<AudioObjectPropertyAddress>);
pub(super) const NON_INTERLEAVED: u32 = 1 << 5; // kAudioFormatFlagIsNonInterleaved

/// Opts out of App Nap and idle sleep for as long as it lives: without it macOS naps a hidden window and the
/// frames stop, and a recording must not end at an idle sleep.
pub struct Awake(Retained<ProtocolObject<dyn NSObjectProtocol>>);

unsafe impl Send for Awake {}

impl Awake {
    pub fn new() -> Awake {
        Awake(
            NSProcessInfo::processInfo().beginActivityWithOptions_reason(
                NSActivityOptions::UserInitiated,
                &NSString::from_str("Recording a meeting"),
            ),
        )
    }
}

impl Drop for Awake {
    fn drop(&mut self) {
        unsafe { NSProcessInfo::processInfo().endActivity(&self.0) }
    }
}

/// Reads the buffers of an input callback down to one mono channel. `planar` buffers hold one channel each,
/// interleaved ones `mNumberChannels`, capped at the format's channel count.
pub(super) unsafe fn mono_of(list: &AudioBufferList, planar: bool, channels: usize) -> Vec<f32> {
    let bufs = slice::from_raw_parts(list.mBuffers.as_ptr(), list.mNumberBuffers as usize);
    let planes: Vec<(&[f32], usize)> = bufs
        .iter()
        .filter(|b| !b.mData.is_null())
        .map(|b| {
            let n = b.mDataByteSize as usize / 4;
            let ch = if planar {
                1
            } else {
                (b.mNumberChannels as usize).max(1).min(channels.max(1))
            };
            (slice::from_raw_parts(b.mData as *const f32, n), ch)
        })
        .collect();
    downmix(&planes)
}

static FORMAT_ADDRESS: AudioObjectPropertyAddress = AudioObjectPropertyAddress {
    mSelector: kAudioTapPropertyFormat,
    mScope: kAudioObjectPropertyScopeGlobal,
    mElement: kAudioObjectPropertyElementMain,
};

/// The callback reads 32-bit float samples; refuse anything else instead of playing noise.
pub(super) fn check_format(f: &AudioStreamBasicDescription) -> Result<(), String> {
    if f.mFormatID == 0x6c70636d && f.mFormatFlags & 1 != 0 && f.mBitsPerChannel == 32 {
        Ok(())
    } else {
        Err("the audio tap is not 32-bit float".into())
    }
}

pub(super) fn check(what: &str, status: i32) -> Result<(), String> {
    if status == 0 {
        Ok(())
    } else {
        Err(format!("{what} failed (OSStatus {status})"))
    }
}

/// A running capture. Dropping it stops the callback and tears everything down in reverse order.
pub struct Tap {
    tap: AudioObjectID,
    aggregate: Option<AudioObjectID>,
    io_proc: AudioDeviceIOProcID,
    _block: Option<RcBlock<IoBlock>>,
    // Re-reads the tap's sample rate when it changes (a Bluetooth headset can switch profile mid-call).
    listener: Option<RcBlock<ListenerBlock>>,
    _awake: Option<Awake>,
}

// The Objective-C handles are only touched from `start` and `drop`, never from the audio thread.
unsafe impl Send for Tap {}

impl Tap {
    /// `on_frame` gets 1600 mono i16 samples at 16 kHz. It runs on the audio thread, so keep it quick.
    pub fn start(on_frame: impl Fn(&[i16]) + Send + Sync + 'static) -> Result<Tap, String> {
        unsafe {
            let desc = CATapDescription::initStereoGlobalTapButExcludeProcesses(
                CATapDescription::alloc(),
                &NSArray::<NSNumber>::new(),
            );
            desc.setName(&NSString::from_str("Recapz"));
            desc.setPrivate(true);
            desc.setMuteBehavior(CATapMuteBehavior::Unmuted);

            let mut me = Tap {
                tap: 0,
                aggregate: None,
                io_proc: None,
                _block: None,
                listener: None,
                _awake: None,
            };
            check(
                "create the audio tap",
                AudioHardwareCreateProcessTap(Some(&desc), &mut me.tap),
            )?;
            let format = format_of(me.tap)?;
            check_format(&format)?;
            let channels = format.mChannelsPerFrame as usize;
            let planar = format.mFormatFlags & NON_INTERLEAVED != 0;

            let uid = NSUUID::UUID().UUIDString();
            let tap_uid = desc.UUID().UUIDString();
            let tap_entry = dict(&[("uid", obj(tap_uid)), ("drift", obj(NSNumber::new_i32(1)))]);
            let aggregate_desc = dict(&[
                ("name", obj(NSString::from_str("Recapz tap"))),
                ("uid", obj(uid)),
                ("private", obj(NSNumber::new_i32(1))),
                ("taps", obj(NSArray::from_retained_slice(&[tap_entry]))),
            ]);
            let mut aggregate = 0;
            check(
                "create the audio device",
                AudioHardwareCreateAggregateDevice(
                    &*(Retained::as_ptr(&aggregate_desc) as *const CFDictionary),
                    NonNull::from(&mut aggregate),
                ),
            )?;
            me.aggregate = Some(aggregate);

            let rate = Arc::new(AtomicU32::new(format.mSampleRate as u32));
            let tap_id = me.tap;
            let listener_rate = rate.clone();
            let listener = RcBlock::new(move |_: u32, _: NonNull<AudioObjectPropertyAddress>| {
                if let Ok(f) = format_of(tap_id) {
                    listener_rate.store(f.mSampleRate as u32, Ordering::Relaxed);
                }
            });
            check(
                "watch the tap format",
                AudioObjectAddPropertyListenerBlock(
                    me.tap,
                    NonNull::from(&FORMAT_ADDRESS),
                    None,
                    RcBlock::as_ptr(&listener),
                ),
            )?;
            me.listener = Some(listener);

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
                    framer.push(&mono, &on_frame);
                },
            );
            check(
                "register the audio callback",
                AudioDeviceCreateIOProcIDWithBlock(
                    NonNull::from(&mut me.io_proc),
                    aggregate,
                    None,
                    RcBlock::as_ptr(&block),
                ),
            )?;
            me._block = Some(block);
            check(
                "start the audio device",
                AudioDeviceStart(aggregate, me.io_proc),
            )?;

            me._awake = Some(Awake::new());
            Ok(me)
        }
    }
}

impl Drop for Tap {
    fn drop(&mut self) {
        unsafe {
            if let Some(a) = self.aggregate {
                if self.io_proc.is_some() {
                    AudioDeviceStop(a, self.io_proc);
                    AudioDeviceDestroyIOProcID(a, self.io_proc);
                }
                AudioHardwareDestroyAggregateDevice(a);
            }
            if let Some(l) = &self.listener {
                AudioObjectRemovePropertyListenerBlock(
                    self.tap,
                    NonNull::from(&FORMAT_ADDRESS),
                    None,
                    RcBlock::as_ptr(l),
                );
            }
            if self.tap != 0 {
                AudioHardwareDestroyProcessTap(self.tap);
            }
        }
    }
}

/// Whether macOS lets us create a tap: the permission prompt appears the first time this runs.
pub fn can_tap() -> Result<(), String> {
    unsafe {
        let desc = CATapDescription::initStereoGlobalTapButExcludeProcesses(
            CATapDescription::alloc(),
            &NSArray::<NSNumber>::new(),
        );
        desc.setPrivate(true);
        let mut id = 0;
        check(
            "create the audio tap",
            AudioHardwareCreateProcessTap(Some(&desc), &mut id),
        )?;
        AudioHardwareDestroyProcessTap(id);
        Ok(())
    }
}

unsafe fn format_of(tap: AudioObjectID) -> Result<AudioStreamBasicDescription, String> {
    let mut out = std::mem::zeroed::<AudioStreamBasicDescription>();
    let mut size = size_of::<AudioStreamBasicDescription>() as u32;
    check(
        "read the tap format",
        AudioObjectGetPropertyData(
            tap,
            NonNull::from(&FORMAT_ADDRESS),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::from(&mut out).cast::<c_void>(),
        ),
    )?;
    Ok(out)
}

fn obj<T: objc2::Message>(x: Retained<T>) -> Retained<AnyObject> {
    unsafe { Retained::cast_unchecked(x) }
}

/// An NSDictionary (toll-free bridged to the CFDictionary Core Audio wants) keyed by the HAL's string keys.
fn dict(pairs: &[(&str, Retained<AnyObject>)]) -> Retained<NSDictionary<NSString, AnyObject>> {
    let keys: Vec<_> = pairs.iter().map(|(k, _)| NSString::from_str(k)).collect();
    let refs: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
    let vals: Vec<_> = pairs.iter().map(|(_, v)| v.clone()).collect();
    NSDictionary::from_retained_objects(&refs, &vals)
}
