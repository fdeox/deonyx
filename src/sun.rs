// Sunrise and sunset for a place, worked out on the computer (the NOAA
// formulas), so darkening can follow the sun without asking the internet or
// Windows for the user's location.

use std::f64::consts::PI;

use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows::Win32::System::SystemInformation::{GetLocalTime, GetSystemTime};
use windows::Win32::System::Time::SystemTimeToFileTime;

/// Today's sunrise and sunset in local minutes since midnight, or what the
/// sun does all day near the poles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Day {
    Times { rise: u32, set: u32 },
    /// The sun stays up (midsummer near the poles).
    Light,
    /// It never comes up (midwinter there).
    Dark,
}

fn ticks_of(t: &SYSTEMTIME) -> i64 {
    let mut ft = FILETIME::default();
    unsafe {
        let _ = SystemTimeToFileTime(t, &mut ft);
    }
    ((ft.dwHighDateTime as u64) << 32 | ft.dwLowDateTime as u64) as i64
}

/// Minutes the local clock is ahead of UTC right now (summer time included).
/// Time zones differ by whole quarter hours.
fn utc_offset() -> i64 {
    let (local, utc) = unsafe { (GetLocalTime(), GetSystemTime()) };
    let minutes = (ticks_of(&local) - ticks_of(&utc)) as f64 / 600_000_000.0;
    (minutes / 15.0).round() as i64 * 15
}

fn day_of_year(y: i32, m: u32, d: u32) -> u32 {
    const BEFORE: [u32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    BEFORE[(m.clamp(1, 12) - 1) as usize] + d + u32::from(leap && m > 2)
}

/// Sunrise and sunset on a day of the year, as minutes from midnight in a
/// time zone `offset` minutes ahead of UTC.
pub fn day(lat: f64, lon: f64, doy: u32, offset: i64) -> Day {
    let g = 2.0 * PI / 365.0 * (doy as f64 - 1.0);
    let eqtime = 229.18
        * (0.000075 + 0.001868 * g.cos() - 0.032077 * g.sin() - 0.014615 * (2.0 * g).cos() - 0.040849 * (2.0 * g).sin());
    let decl = 0.006918 - 0.399912 * g.cos() + 0.070257 * g.sin() - 0.006758 * (2.0 * g).cos() + 0.000907 * (2.0 * g).sin()
        - 0.002697 * (3.0 * g).cos()
        + 0.00148 * (3.0 * g).sin();
    let phi = lat.to_radians();
    // The sun's centre 0.833 degrees below the horizon: refraction and its size.
    let c = (90.833f64).to_radians().cos() / (phi.cos() * decl.cos()) - phi.tan() * decl.tan();
    if c > 1.0 {
        return Day::Dark;
    }
    if c < -1.0 {
        return Day::Light;
    }
    let ha = c.acos().to_degrees();
    let local = |utc: f64| ((utc + offset as f64).round() as i64).rem_euclid(1440) as u32;
    Day::Times { rise: local(720.0 - 4.0 * (lon + ha) - eqtime), set: local(720.0 - 4.0 * (lon - ha) - eqtime) }
}

/// Today, where the user is.
pub fn today(lat: f64, lon: f64) -> Day {
    let t = unsafe { GetLocalTime() };
    day(lat, lon, day_of_year(t.wYear as i32, t.wMonth as u32, t.wDay as u32), utc_offset())
}

/// Whether it's dark outside now, between sunset and sunrise.
pub fn is_night(lat: f64, lon: f64, now: u32) -> bool {
    match today(lat, lon) {
        Day::Times { rise, set } => {
            if rise <= set {
                now >= set || now < rise
            } else {
                now >= set && now < rise
            }
        }
        Day::Light => false,
        Day::Dark => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn istanbul_in_june() {
        // Istanbul, 21 June, UTC+3: sunrise about 05:31, sunset about 20:39.
        let Day::Times { rise, set } = day(41.01, 28.98, day_of_year(2026, 6, 21), 180) else { panic!() };
        assert!((rise as i32 - (5 * 60 + 31)).abs() <= 4, "rise {rise}");
        assert!((set as i32 - (20 * 60 + 39)).abs() <= 4, "set {set}");
    }

    #[test]
    fn the_poles() {
        assert_eq!(day(78.2, 15.6, day_of_year(2026, 6, 21), 120), Day::Light);
        assert_eq!(day(78.2, 15.6, day_of_year(2026, 12, 21), 60), Day::Dark);
    }
}
