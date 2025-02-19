use stm32_metapac::rtc::vals::{Calp, Calw16, Calw8, Fmt, Init, Key, Osel, Pol, TampalrmPu, TampalrmType};

use super::{sealed, Instance, RtcCalibrationCyclePeriod, RtcConfig};
use crate::pac::rtc::Rtc;

impl<'d, T: Instance> super::Rtc<'d, T> {
    /// Applies the RTC config
    /// It this changes the RTC clock source the time will be reset
    pub(super) fn apply_config(&mut self, rtc_config: RtcConfig) {
        // Unlock the backup domain
        unsafe {
            #[cfg(any(rtc_v3u5, rcc_g0, rcc_g4))]
            use crate::pac::rcc::vals::Rtcsel;
            #[cfg(not(any(rtc_v3u5, rcc_g0, rcc_g4, rcc_wl5, rcc_wle)))]
            use crate::pac::rtc::vals::Rtcsel;

            #[cfg(not(any(rtc_v3u5, rcc_wl5, rcc_wle)))]
            {
                crate::pac::PWR.cr1().modify(|w| w.set_dbp(true));
                while !crate::pac::PWR.cr1().read().dbp() {}
            }
            #[cfg(any(rcc_wl5, rcc_wle))]
            {
                use crate::pac::pwr::vals::Dbp;

                crate::pac::PWR.cr1().modify(|w| w.set_dbp(Dbp::ENABLED));
                while crate::pac::PWR.cr1().read().dbp() != Dbp::ENABLED {}
            }

            let reg = crate::pac::RCC.bdcr().read();
            assert!(!reg.lsecsson(), "RTC is not compatible with LSE CSS, yet.");

            let config_rtcsel = rtc_config.clock_config as u8;
            #[cfg(not(any(rcc_wl5, rcc_wle)))]
            let config_rtcsel = Rtcsel(config_rtcsel);

            if !reg.rtcen() || reg.rtcsel() != config_rtcsel {
                crate::pac::RCC.bdcr().modify(|w| w.set_bdrst(true));

                crate::pac::RCC.bdcr().modify(|w| {
                    // Reset
                    w.set_bdrst(false);

                    // Select RTC source
                    w.set_rtcsel(config_rtcsel);

                    w.set_rtcen(true);

                    // Restore bcdr
                    w.set_lscosel(reg.lscosel());
                    w.set_lscoen(reg.lscoen());

                    w.set_lseon(reg.lseon());
                    w.set_lsedrv(reg.lsedrv());
                    w.set_lsebyp(reg.lsebyp());
                });
            }
        }

        self.write(true, |rtc| {
            unsafe {
                rtc.cr().modify(|w| {
                    w.set_fmt(Fmt::TWENTYFOURHOUR);
                    w.set_osel(Osel::DISABLED);
                    w.set_pol(Pol::HIGH);
                });

                rtc.prer().modify(|w| {
                    w.set_prediv_s(rtc_config.sync_prescaler);
                    w.set_prediv_a(rtc_config.async_prescaler);
                });

                // TODO: configuration for output pins
                rtc.cr().modify(|w| {
                    w.set_out2en(false);
                    w.set_tampalrm_type(TampalrmType::PUSHPULL);
                    w.set_tampalrm_pu(TampalrmPu::NOPULLUP);
                });
            }
        });

        self.rtc_config = rtc_config;
    }

    const RTC_CALR_MIN_PPM: f32 = -487.1;
    const RTC_CALR_MAX_PPM: f32 = 488.5;
    const RTC_CALR_RESOLUTION_PPM: f32 = 0.9537;

    /// Calibrate the clock drift.
    ///
    /// `clock_drift` can be adjusted from -487.1 ppm to 488.5 ppm and is clamped to this range.
    ///
    /// ### Note
    ///
    /// To perform a calibration when `async_prescaler` is less then 3, `sync_prescaler`
    /// has to be reduced accordingly (see RM0351 Rev 9, sec 38.3.12).
    pub fn calibrate(&mut self, mut clock_drift: f32, period: RtcCalibrationCyclePeriod) {
        if clock_drift < Self::RTC_CALR_MIN_PPM {
            clock_drift = Self::RTC_CALR_MIN_PPM;
        } else if clock_drift > Self::RTC_CALR_MAX_PPM {
            clock_drift = Self::RTC_CALR_MAX_PPM;
        }

        clock_drift = clock_drift / Self::RTC_CALR_RESOLUTION_PPM;

        self.write(false, |rtc| {
            unsafe {
                rtc.calr().write(|w| {
                    match period {
                        RtcCalibrationCyclePeriod::Seconds8 => {
                            w.set_calw8(Calw8::EIGHTSECONDS);
                        }
                        RtcCalibrationCyclePeriod::Seconds16 => {
                            w.set_calw16(Calw16::SIXTEENSECONDS);
                        }
                        RtcCalibrationCyclePeriod::Seconds32 => {
                            // Set neither `calw8` nor `calw16` to use 32 seconds
                        }
                    }

                    // Extra pulses during calibration cycle period: CALP * 512 - CALM
                    //
                    // CALP sets whether pulses are added or omitted.
                    //
                    // CALM contains how many pulses (out of 512) are masked in a
                    // given calibration cycle period.
                    if clock_drift > 0.0 {
                        // Maximum (about 512.2) rounds to 512.
                        clock_drift += 0.5;

                        // When the offset is positive (0 to 512), the opposite of
                        // the offset (512 - offset) is masked, i.e. for the
                        // maximum offset (512), 0 pulses are masked.
                        w.set_calp(Calp::INCREASEFREQ);
                        w.set_calm(512 - clock_drift as u16);
                    } else {
                        // Minimum (about -510.7) rounds to -511.
                        clock_drift -= 0.5;

                        // When the offset is negative or zero (-511 to 0),
                        // the absolute offset is masked, i.e. for the minimum
                        // offset (-511), 511 pulses are masked.
                        w.set_calp(Calp::NOCHANGE);
                        w.set_calm((clock_drift * -1.0) as u16);
                    }
                });
            }
        })
    }

    pub(super) fn write<F, R>(&mut self, init_mode: bool, f: F) -> R
    where
        F: FnOnce(&crate::pac::rtc::Rtc) -> R,
    {
        let r = T::regs();
        // Disable write protection.
        // This is safe, as we're only writin the correct and expected values.
        unsafe {
            r.wpr().write(|w| w.set_key(Key::DEACTIVATE1));
            r.wpr().write(|w| w.set_key(Key::DEACTIVATE2));

            if init_mode && !r.icsr().read().initf() {
                r.icsr().modify(|w| w.set_init(Init::INITMODE));
                // wait till init state entered
                // ~2 RTCCLK cycles
                while !r.icsr().read().initf() {}
            }
        }

        let result = f(&r);

        unsafe {
            if init_mode {
                r.icsr().modify(|w| w.set_init(Init::FREERUNNINGMODE)); // Exits init mode
            }

            // Re-enable write protection.
            // This is safe, as the field accepts the full range of 8-bit values.
            r.wpr().write(|w| w.set_key(Key::ACTIVATE));
        }
        result
    }
}

impl sealed::Instance for crate::peripherals::RTC {
    const BACKUP_REGISTER_COUNT: usize = 32;

    fn read_backup_register(_rtc: &Rtc, register: usize) -> Option<u32> {
        if register < Self::BACKUP_REGISTER_COUNT {
            //Some(rtc.bkpr()[register].read().bits())
            None // RTC3 backup registers come from the TAMP peripe=heral, not RTC. Not() even in the L412 PAC
        } else {
            None
        }
    }

    fn write_backup_register(_rtc: &Rtc, register: usize, _value: u32) {
        if register < Self::BACKUP_REGISTER_COUNT {
            // RTC3 backup registers come from the TAMP peripe=heral, not RTC. Not() even in the L412 PAC
            //unsafe { self.rtc.bkpr()[register].write(|w| w.bits(value)) }
        }
    }
}

impl Instance for crate::peripherals::RTC {}
