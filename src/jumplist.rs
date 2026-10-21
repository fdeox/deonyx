// Right-click Deonyx on the taskbar or in Start: switch darkening, pause it
// for an hour, open the settings. Windows starts deonyx.exe with those
// arguments, and the command line (cli.rs) hands them to the running copy.

use windows::core::{w, Interface, Result, HSTRING};
use windows::Win32::Storage::EnhancedStorage::PKEY_Title;
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PROPVARIANT};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::System::Variant::VT_LPWSTR;
use windows::Win32::UI::Shell::Common::{IObjectArray, IObjectCollection};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{
    DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellLinkW, SHStrDupW,
    SetCurrentProcessExplicitAppUserModelID, ShellLink,
};

/// The same id the installer gives the Start menu shortcut (the bundle id),
/// so Windows puts the tasks on it.
const APP_ID: windows::core::PCWSTR = w!("com.fdeox.deonyx");

/// Call early, before any window opens.
pub fn claim_app_id() {
    unsafe {
        let _ = SetCurrentProcessExplicitAppUserModelID(APP_ID);
    }
}

/// The three tasks' titles: switch, pause for an hour, settings.
fn titles(lang: &str) -> [&'static str; 3] {
    match lang {
        "tr" => ["Karartmayı aç/kapat", "1 saat duraklat", "Ayarlar"],
        "de" => ["Abdunkeln an/aus", "1 Stunde pausieren", "Einstellungen"],
        "fr" => ["Activer/désactiver l'assombrissement", "Pause d'une heure", "Paramètres"],
        "es" => ["Activar/desactivar el oscurecimiento", "Pausar 1 hora", "Ajustes"],
        "it" => ["Attiva/disattiva l'oscuramento", "Pausa di 1 ora", "Impostazioni"],
        "pt" => ["Ligar/desligar o escurecimento", "Pausar por 1 hora", "Configurações"],
        "ru" => ["Включить/выключить затемнение", "Пауза на 1 час", "Настройки"],
        "uk" => ["Увімкнути/вимкнути затемнення", "Пауза на 1 годину", "Налаштування"],
        "nl" => ["Donker maken aan/uit", "1 uur pauzeren", "Instellingen"],
        "ja" => ["暗くする機能のオン/オフ", "1 時間一時停止", "設定"],
        "ko" => ["어둡게 하기 켜기/끄기", "1시간 일시 중지", "설정"],
        "zh" => ["开启/关闭调暗", "暂停 1 小时", "设置"],
        _ => ["Turn darkening on or off", "Pause for 1 hour", "Settings"],
    }
}

fn task(exe: &HSTRING, args: &str, title: &str) -> Result<IShellLinkW> {
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(exe)?;
        link.SetArguments(&HSTRING::from(args))?;
        link.SetIconLocation(exe, 0)?;
        let props: IPropertyStore = link.cast()?;
        // The title lives in COM memory, freed by PropVariantClear: the
        // store copies it, and anything else corrupts the heap.
        let mut v = PROPVARIANT::default();
        (*v.Anonymous.Anonymous).vt = VT_LPWSTR;
        (*v.Anonymous.Anonymous).Anonymous.pwszVal = SHStrDupW(&HSTRING::from(title))?;
        let set = props.SetValue(&PKEY_Title, &v);
        let _ = PropVariantClear(&mut v);
        set?;
        props.Commit()?;
        Ok(link)
    }
}

/// Puts the tasks on the shortcut with that id, in the settings' language.
pub fn set(app_id: &str, lang: &str) -> Result<()> {
    let exe = HSTRING::from(std::env::current_exe().map_err(|_| windows::core::Error::from_win32())?.as_os_str());
    let t = titles(lang);
    unsafe {
        let list: ICustomDestinationList = CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER)?;
        list.SetAppID(&HSTRING::from(app_id))?;
        let mut slots = 0u32;
        let _removed: IObjectArray = list.BeginList(&mut slots)?;
        let tasks: IObjectCollection = CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)?;
        tasks.AddObject(&task(&exe, "--toggle", t[0])?)?;
        tasks.AddObject(&task(&exe, "--pause 60", t[1])?)?;
        tasks.AddObject(&task(&exe, "--settings", t[2])?)?;
        list.AddUserTasks(&tasks.cast::<IObjectArray>()?)?;
        list.CommitList()?;
    }
    Ok(())
}

/// Takes the tasks off again (for the test run's own id).
pub fn forget(app_id: &str) -> Result<()> {
    unsafe {
        let list: ICustomDestinationList = CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER)?;
        list.DeleteList(&HSTRING::from(app_id))
    }
}

#[cfg(test)]
mod tests {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};

    // Under a test id, so the real Deonyx's list stays as it is.
    #[test]
    fn sets_and_forgets_tasks() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        super::set("com.fdeox.deonyx.test", "tr").unwrap();
        super::forget("com.fdeox.deonyx.test").unwrap();
    }
}
