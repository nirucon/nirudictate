use super::*;

fn p(text: &str) -> Option<Intent> {
    parse(text, Lang::Sv).map(|p| p.intent)
}

fn p_en(text: &str) -> Option<Intent> {
    parse(text, Lang::En).map(|p| p.intent)
}

/// (what was heard, what it must mean). Swedish preference, like the
/// default configuration; the English column of a Swedish user.
const CASES: &[(&str, Intent)] = &[
    // terminal
    ("öppna terminalen", Intent::Terminal),
    ("Öppna terminalen.", Intent::Terminal),
    ("Öppna terminal", Intent::Terminal),
    ("öppna en terminal", Intent::Terminal),
    ("öppna ett terminalfönster", Intent::Terminal),
    ("starta terminalen", Intent::Terminal),
    ("kör terminal", Intent::Terminal),
    ("ny terminal", Intent::Terminal),
    ("terminal", Intent::Terminal),
    ("Terminalen!", Intent::Terminal),
    ("kan du öppna terminalen", Intent::Terminal),
    ("kan du öppna terminalen tack", Intent::Terminal),
    ("Kan du snälla öppna terminalen?", Intent::Terminal),
    ("skulle du kunna öppna en terminal", Intent::Terminal),
    ("eh, öppna terminalen", Intent::Terminal),
    ("Öh... öppna, eh, terminalen.", Intent::Terminal),
    ("hej öppna terminalen", Intent::Terminal),
    ("öppna terminalen nu", Intent::Terminal),
    ("öppna konsolen", Intent::Terminal),
    ("open terminal", Intent::Terminal),
    ("Open a terminal.", Intent::Terminal),
    ("Open the terminal, please.", Intent::Terminal),
    ("please open a new terminal", Intent::Terminal),
    ("could you open a terminal for me", Intent::Terminal),
    ("can you launch the terminal", Intent::Terminal),
    ("launch terminal", Intent::Terminal),
    ("start a terminal", Intent::Terminal),
    ("um, open terminal", Intent::Terminal),
    ("Hey, open a terminal window.", Intent::Terminal),
    ("open the console", Intent::Terminal),
    ("öppna appen terminal", Intent::Terminal),
    // browser
    ("öppna webbläsaren", Intent::Browser),
    ("Öppna webbläsaren.", Intent::Browser),
    ("öppna webbläsarn", Intent::Browser),
    ("öppna webb läsaren", Intent::Browser),
    ("öppna en webbläsare", Intent::Browser),
    ("starta webbläsaren", Intent::Browser),
    ("webbläsare", Intent::Browser),
    ("öppna browsern", Intent::Browser),
    ("öppna internet", Intent::Browser),
    ("kan du öppna webbläsaren tack", Intent::Browser),
    ("open browser", Intent::Browser),
    ("open the browser", Intent::Browser),
    ("Open a web browser.", Intent::Browser),
    ("launch the browser please", Intent::Browser),
    ("browser", Intent::Browser),
    ("open the internet", Intent::Browser),
    // workspaces, Swedish numbers
    ("arbetsyta ett", Intent::Workspace(1)),
    ("arbetsyta en", Intent::Workspace(1)),
    ("gå till arbetsyta två", Intent::Workspace(2)),
    ("gå till arbetsyta tre", Intent::Workspace(3)),
    ("Gå till arbetsyta 3.", Intent::Workspace(3)),
    ("gå till arbetsytan fyra", Intent::Workspace(4)),
    ("byt till arbetsyta fem", Intent::Workspace(5)),
    ("växla till arbetsyta sex", Intent::Workspace(6)),
    ("arbetsyta sju", Intent::Workspace(7)),
    ("visa arbetsyta åtta", Intent::Workspace(8)),
    ("arbetsyta nio", Intent::Workspace(9)),
    ("arbetsyta tio", Intent::Workspace(10)),
    ("arbetsyta 10", Intent::Workspace(10)),
    ("arbetsyta nummer tre", Intent::Workspace(3)),
    ("gå till arbetsyta nr 4", Intent::Workspace(4)),
    ("gå till arbetsyta 3:an", Intent::Workspace(3)),
    ("gå till arbetsyta trean", Intent::Workspace(3)),
    ("gå till tredje arbetsytan", Intent::Workspace(3)),
    ("gå till den andra arbetsytan", Intent::Workspace(2)),
    ("byt till första skrivbordet", Intent::Workspace(1)),
    ("skrivbord fem", Intent::Workspace(5)),
    ("gå till skrivbordet två", Intent::Workspace(2)),
    ("växla till workspace fem", Intent::Workspace(5)),
    ("byt till workspace 2", Intent::Workspace(2)),
    ("kan du gå till arbetsyta tre", Intent::Workspace(3)),
    ("Kan du byta till arbetsyta två?", Intent::Workspace(2)),
    ("hoppa till arbetsyta sju", Intent::Workspace(7)),
    // workspaces, English
    ("workspace one", Intent::Workspace(1)),
    ("go to workspace two", Intent::Workspace(2)),
    ("go to workspace three", Intent::Workspace(3)),
    ("Go to workspace 3.", Intent::Workspace(3)),
    ("switch to workspace four", Intent::Workspace(4)),
    ("switch to desktop five", Intent::Workspace(5)),
    ("workspace six", Intent::Workspace(6)),
    ("workspace seven please", Intent::Workspace(7)),
    ("go to workspace eight", Intent::Workspace(8)),
    ("workspace nine", Intent::Workspace(9)),
    ("workspace ten", Intent::Workspace(10)),
    ("workspace number 4", Intent::Workspace(4)),
    ("go to the third workspace", Intent::Workspace(3)),
    ("go to the second desktop", Intent::Workspace(2)),
    ("show workspace 1", Intent::Workspace(1)),
    ("go to work space 2", Intent::Workspace(2)),
    // Whisper writes homophones
    ("go to workspace to", Intent::Workspace(2)),
    ("go to workspace too", Intent::Workspace(2)),
    ("workspace for", Intent::Workspace(4)),
    ("go to workspace won", Intent::Workspace(1)),
    ("go to workspace 3rd", Intent::Workspace(3)),
    ("go to workspace tree", Intent::Workspace(3)),
    // move a window
    (
        "flytta fönstret till arbetsyta tre",
        Intent::MoveToWorkspace(3),
    ),
    (
        "Flytta fönstret till arbetsyta 2.",
        Intent::MoveToWorkspace(2),
    ),
    ("flytta till arbetsytan fyra", Intent::MoveToWorkspace(4)),
    (
        "skicka fönstret till arbetsyta fem",
        Intent::MoveToWorkspace(5),
    ),
    (
        "flytta det här fönstret till skrivbord 6",
        Intent::MoveToWorkspace(6),
    ),
    (
        "flytta fönstret till tredje arbetsytan",
        Intent::MoveToWorkspace(3),
    ),
    ("flytta fönstret till trean", Intent::MoveToWorkspace(3)),
    ("flytta den till arbetsyta sju", Intent::MoveToWorkspace(7)),
    (
        "kan du flytta fönstret till arbetsyta två",
        Intent::MoveToWorkspace(2),
    ),
    (
        "flytta fönstret till workspace 9",
        Intent::MoveToWorkspace(9),
    ),
    ("move window to workspace three", Intent::MoveToWorkspace(3)),
    (
        "move this window to workspace 2",
        Intent::MoveToWorkspace(2),
    ),
    (
        "Move the window to workspace five.",
        Intent::MoveToWorkspace(5),
    ),
    ("send it to desktop four", Intent::MoveToWorkspace(4)),
    ("move to workspace ten", Intent::MoveToWorkspace(10)),
    (
        "move the window to the third workspace",
        Intent::MoveToWorkspace(3),
    ),
    ("move window to workspace to", Intent::MoveToWorkspace(2)),
    // next and previous
    ("nästa arbetsyta", Intent::NextWorkspace),
    ("gå till nästa arbetsyta", Intent::NextWorkspace),
    ("byt till nästa skrivbord", Intent::NextWorkspace),
    ("föregående arbetsyta", Intent::PrevWorkspace),
    ("gå till förra arbetsytan", Intent::PrevWorkspace),
    ("gå tillbaka till förra arbetsytan", Intent::PrevWorkspace),
    ("next workspace", Intent::NextWorkspace),
    ("go to the next workspace", Intent::NextWorkspace),
    ("previous workspace", Intent::PrevWorkspace),
    ("switch to the previous desktop", Intent::PrevWorkspace),
    // windows
    ("stäng fönstret", Intent::CloseWindow),
    ("Stäng fönstret.", Intent::CloseWindow),
    ("stäng det här fönstret", Intent::CloseWindow),
    ("kan du stänga fönstret", Intent::CloseWindow),
    ("stäng appen", Intent::CloseWindow),
    ("close window", Intent::CloseWindow),
    ("close the window", Intent::CloseWindow),
    ("Close this window, please.", Intent::CloseWindow),
    ("close the app", Intent::CloseWindow),
    ("helskärm", Intent::Fullscreen),
    ("Helskärm.", Intent::Fullscreen),
    ("fullskärm", Intent::Fullscreen),
    ("växla helskärm", Intent::Fullscreen),
    ("gör fönstret till helskärm", Intent::Fullscreen),
    ("avsluta helskärm", Intent::Fullscreen),
    ("fullscreen", Intent::Fullscreen),
    ("full screen", Intent::Fullscreen),
    ("toggle fullscreen", Intent::Fullscreen),
    ("make it full screen", Intent::Fullscreen),
    ("make it full-screen", Intent::Fullscreen),
    ("flytande fönster", Intent::Float),
    ("gör fönstret flytande", Intent::Float),
    ("float", Intent::Float),
    ("toggle floating", Intent::Float),
    ("make the window floating", Intent::Float),
    // layouts
    ("layout monocle", Intent::Layout("monocle")),
    ("byt till monokel", Intent::Layout("monocle")),
    ("monokel", Intent::Layout("monocle")),
    ("byt layout till monokel", Intent::Layout("monocle")),
    ("växla till kakel", Intent::Layout("tile")),
    ("kakel layout", Intent::Layout("tile")),
    ("sida vid sida", Intent::Layout("tile")),
    ("flytande layout", Intent::Layout("floating")),
    ("byt till flytande läge", Intent::Layout("floating")),
    ("centrerad layout", Intent::Layout("centered")),
    ("layout spiral", Intent::Layout("fibonacci")),
    ("rullande layout", Intent::Layout("scroll")),
    ("nästa layout", Intent::NextLayout),
    ("byt layout", Intent::NextLayout),
    ("switch to monocle", Intent::Layout("monocle")),
    ("monocle layout", Intent::Layout("monocle")),
    ("tiling layout", Intent::Layout("tile")),
    ("switch to tile layout", Intent::Layout("tile")),
    ("floating layout", Intent::Layout("floating")),
    ("centered layout", Intent::Layout("centered")),
    ("fibonacci", Intent::Layout("fibonacci")),
    ("scrolling layout", Intent::Layout("scroll")),
    ("next layout", Intent::NextLayout),
    ("change layout", Intent::NextLayout),
    // lock
    ("lås skärmen", Intent::Lock),
    ("Lås skärmen.", Intent::Lock),
    ("lås datorn", Intent::Lock),
    ("lås", Intent::Lock),
    ("låsskärm", Intent::Lock),
    ("kan du låsa skärmen", Intent::Lock),
    ("lås skärmen tack", Intent::Lock),
    ("lock screen", Intent::Lock),
    ("Lock the screen.", Intent::Lock),
    ("lock my computer", Intent::Lock),
    ("lock", Intent::Lock),
    ("please lock the screen", Intent::Lock),
    ("lock the computer now", Intent::Lock),
    // launcher, search, hints, note window, overview
    ("öppna startmenyn", Intent::Launcher),
    ("visa appar", Intent::Launcher),
    ("programmenyn", Intent::Launcher),
    ("öppna launchern", Intent::Launcher),
    ("open the launcher", Intent::Launcher),
    ("show apps", Intent::Launcher),
    ("open the start menu", Intent::Launcher),
    ("sök", Intent::SearchUi),
    ("öppna sökning", Intent::SearchUi),
    ("search", Intent::SearchUi),
    ("open search", Intent::SearchUi),
    ("fönsterhopp", Intent::WindowJump),
    ("hoppa till fönster", Intent::WindowJump),
    ("window jump", Intent::WindowJump),
    ("jump to a window", Intent::WindowJump),
    ("window hints", Intent::WindowJump),
    ("ny anteckning", Intent::QuickNoteUi),
    ("anteckning", Intent::QuickNoteUi),
    ("snabbanteckning", Intent::QuickNoteUi),
    ("new note", Intent::QuickNoteUi),
    ("take a note", Intent::QuickNoteUi),
    ("quick note", Intent::QuickNoteUi),
    ("översikt", Intent::Overview),
    ("visa översikten", Intent::Overview),
    ("visa alla arbetsytor", Intent::Overview),
    ("overview", Intent::Overview),
    ("show all workspaces", Intent::Overview),
    ("show the overview", Intent::Overview),
    // night light
    ("slå på nattljus", Intent::NightLight(Switch::On)),
    ("slå på nattljuset", Intent::NightLight(Switch::On)),
    ("nattljus på", Intent::NightLight(Switch::On)),
    ("sätt på nattljuset", Intent::NightLight(Switch::On)),
    ("aktivera nattläge", Intent::NightLight(Switch::On)),
    ("stäng av nattljuset", Intent::NightLight(Switch::Off)),
    ("nattljus av", Intent::NightLight(Switch::Off)),
    ("släck nattljuset", Intent::NightLight(Switch::Off)),
    (
        "kan du stänga av nattljuset",
        Intent::NightLight(Switch::Off),
    ),
    ("nattljus", Intent::NightLight(Switch::Toggle)),
    ("växla nattljus", Intent::NightLight(Switch::Toggle)),
    ("turn on night light", Intent::NightLight(Switch::On)),
    ("turn on the night light", Intent::NightLight(Switch::On)),
    ("night light on", Intent::NightLight(Switch::On)),
    ("turn the night light on", Intent::NightLight(Switch::On)),
    ("turn off the night light", Intent::NightLight(Switch::Off)),
    ("night light off", Intent::NightLight(Switch::Off)),
    ("disable night mode", Intent::NightLight(Switch::Off)),
    ("night light", Intent::NightLight(Switch::Toggle)),
    ("toggle the night light", Intent::NightLight(Switch::Toggle)),
    // do not disturb
    ("slå på stör ej", Intent::Dnd(Switch::On)),
    ("stör ej på", Intent::Dnd(Switch::On)),
    ("aktivera stör ej", Intent::Dnd(Switch::On)),
    ("stäng av stör ej", Intent::Dnd(Switch::Off)),
    ("stör ej av", Intent::Dnd(Switch::Off)),
    ("stör ej", Intent::Dnd(Switch::Toggle)),
    ("Stör ej.", Intent::Dnd(Switch::Toggle)),
    ("stör inte", Intent::Dnd(Switch::Toggle)),
    ("turn on do not disturb", Intent::Dnd(Switch::On)),
    ("do not disturb on", Intent::Dnd(Switch::On)),
    ("enable do-not-disturb", Intent::Dnd(Switch::On)),
    ("Don't disturb on.", Intent::Dnd(Switch::On)),
    ("turn off do not disturb", Intent::Dnd(Switch::Off)),
    ("do not disturb off", Intent::Dnd(Switch::Off)),
    ("disable dnd", Intent::Dnd(Switch::Off)),
    ("do not disturb", Intent::Dnd(Switch::Toggle)),
    // monitors
    ("nästa skärm", Intent::NextMonitor),
    ("byt till nästa skärm", Intent::NextMonitor),
    ("gå till andra skärmen", Intent::NextMonitor),
    ("föregående skärm", Intent::PrevMonitor),
    ("gå till förra skärmen", Intent::PrevMonitor),
    ("next monitor", Intent::NextMonitor),
    ("switch to the other screen", Intent::NextMonitor),
    ("focus the next display", Intent::NextMonitor),
    ("previous monitor", Intent::PrevMonitor),
    // volume
    ("höj volymen", Intent::VolumeUp),
    ("Höj volymen.", Intent::VolumeUp),
    ("höj ljudet", Intent::VolumeUp),
    ("höj ljudet lite", Intent::VolumeUp),
    ("volym upp", Intent::VolumeUp),
    ("volymen upp", Intent::VolumeUp),
    ("högre", Intent::VolumeUp),
    ("mer ljud", Intent::VolumeUp),
    ("skruva upp volymen", Intent::VolumeUp),
    ("kan du höja volymen", Intent::VolumeUp),
    ("sänk volymen", Intent::VolumeDown),
    ("sänk ljudet", Intent::VolumeDown),
    ("volym ner", Intent::VolumeDown),
    ("volymen ned", Intent::VolumeDown),
    ("tystare", Intent::VolumeDown),
    ("lägre", Intent::VolumeDown),
    ("kan du sänka volymen lite", Intent::VolumeDown),
    ("volume up", Intent::VolumeUp),
    ("turn up the volume", Intent::VolumeUp),
    ("turn the volume up", Intent::VolumeUp),
    ("turn it up", Intent::VolumeUp),
    ("louder", Intent::VolumeUp),
    ("a bit louder please", Intent::VolumeUp),
    ("increase the volume", Intent::VolumeUp),
    ("volume down", Intent::VolumeDown),
    ("turn down the volume", Intent::VolumeDown),
    ("turn it down", Intent::VolumeDown),
    ("quieter", Intent::VolumeDown),
    ("lower the volume", Intent::VolumeDown),
    ("volym 50 procent", Intent::VolumeSet(50)),
    ("volym 50 %", Intent::VolumeSet(50)),
    ("volym 50%", Intent::VolumeSet(50)),
    ("sätt volymen till 30 procent", Intent::VolumeSet(30)),
    ("volymen på femtio", Intent::VolumeSet(50)),
    ("ställ volymen på tjugofem procent", Intent::VolumeSet(25)),
    ("sätt volymen till tjugo fem procent", Intent::VolumeSet(25)),
    ("sänk volymen till 20", Intent::VolumeSet(20)),
    ("volym hundra procent", Intent::VolumeSet(100)),
    ("volym noll", Intent::VolumeSet(0)),
    ("volume 50 percent", Intent::VolumeSet(50)),
    ("set volume to 30%", Intent::VolumeSet(30)),
    (
        "set the volume to twenty five percent",
        Intent::VolumeSet(25),
    ),
    (
        "set the volume to twenty-five percent",
        Intent::VolumeSet(25),
    ),
    ("Set the volume to 70 %.", Intent::VolumeSet(70)),
    ("volume at 40", Intent::VolumeSet(40)),
    ("volume to 60 percent", Intent::VolumeSet(60)),
    ("set volume to one hundred percent", Intent::VolumeSet(100)),
    ("stäng av ljudet", Intent::Mute),
    ("Stäng av ljudet.", Intent::Mute),
    ("ljud av", Intent::Mute),
    ("tysta ljudet", Intent::Mute),
    ("muta ljudet", Intent::Mute),
    ("mute", Intent::Mute),
    ("mute the sound", Intent::Mute),
    ("sound off", Intent::Mute),
    ("turn off the sound", Intent::Mute),
    ("slå på ljudet", Intent::Unmute),
    ("ljud på", Intent::Unmute),
    ("sätt på ljudet", Intent::Unmute),
    ("unmute", Intent::Unmute),
    ("unmute the sound", Intent::Unmute),
    ("turn the sound back on", Intent::Unmute),
    ("stäng av mikrofonen", Intent::MicMute),
    ("mikrofon av", Intent::MicMute),
    ("muta mikrofonen", Intent::MicMute),
    ("mute the microphone", Intent::MicMute),
    ("mute mic", Intent::MicMute),
    ("turn off the mic", Intent::MicMute),
    ("slå på mikrofonen", Intent::MicUnmute),
    ("mikrofon på", Intent::MicUnmute),
    ("unmute the microphone", Intent::MicUnmute),
    ("turn on the mic", Intent::MicUnmute),
    // brightness
    ("ljusare", Intent::BrightnessUp),
    ("höj ljusstyrkan", Intent::BrightnessUp),
    ("ljusstyrka upp", Intent::BrightnessUp),
    ("gör skärmen ljusare", Intent::BrightnessUp),
    ("mörkare", Intent::BrightnessDown),
    ("sänk ljusstyrkan", Intent::BrightnessDown),
    ("ljusstyrkan ner", Intent::BrightnessDown),
    ("dimma skärmen", Intent::BrightnessDown),
    ("ljusstyrka 50 procent", Intent::BrightnessSet(50)),
    (
        "sätt ljusstyrkan till sjuttio procent",
        Intent::BrightnessSet(70),
    ),
    ("brighter", Intent::BrightnessUp),
    ("brightness up", Intent::BrightnessUp),
    ("increase the brightness", Intent::BrightnessUp),
    ("make the screen brighter", Intent::BrightnessUp),
    ("dimmer", Intent::BrightnessDown),
    ("brightness down", Intent::BrightnessDown),
    ("dim the screen", Intent::BrightnessDown),
    ("set brightness to 30%", Intent::BrightnessSet(30)),
    ("brightness eighty percent", Intent::BrightnessSet(80)),
    // media
    ("spela", Intent::Play),
    ("spela musik", Intent::Play),
    ("fortsätt spela", Intent::Play),
    ("play", Intent::Play),
    ("resume", Intent::Play),
    ("play music", Intent::Play),
    ("pausa", Intent::Pause),
    ("Pausa.", Intent::Pause),
    ("paus", Intent::Pause),
    ("pausa musiken", Intent::Pause),
    ("pause", Intent::Pause),
    ("pause the music", Intent::Pause),
    ("nästa låt", Intent::NextTrack),
    ("Nästa låt.", Intent::NextTrack),
    ("spela nästa låt", Intent::NextTrack),
    ("hoppa över låten", Intent::NextTrack),
    ("skippa", Intent::NextTrack),
    ("next song", Intent::NextTrack),
    ("next track", Intent::NextTrack),
    ("skip this song", Intent::NextTrack),
    ("föregående låt", Intent::PrevTrack),
    ("spela förra låten", Intent::PrevTrack),
    ("previous song", Intent::PrevTrack),
    ("previous track", Intent::PrevTrack),
    ("stoppa musiken", Intent::StopMedia),
    ("stop the music", Intent::StopMedia),
    // screenshots
    ("skärmbild", Intent::Screenshot(Shot::Full)),
    ("ta en skärmbild", Intent::Screenshot(Shot::Full)),
    ("Ta en skärmdump.", Intent::Screenshot(Shot::Full)),
    (
        "ta en skärmdump av hela skärmen",
        Intent::Screenshot(Shot::Full),
    ),
    ("fota skärmen", Intent::Screenshot(Shot::Full)),
    ("ta en skärm bild", Intent::Screenshot(Shot::Full)),
    ("skärmbild av ett område", Intent::Screenshot(Shot::Region)),
    (
        "ta en skärmbild av en del av skärmen",
        Intent::Screenshot(Shot::Region),
    ),
    ("skärmbild av fönstret", Intent::Screenshot(Shot::Window)),
    (
        "ta en skärmdump av det här fönstret",
        Intent::Screenshot(Shot::Window),
    ),
    ("screenshot", Intent::Screenshot(Shot::Full)),
    ("take a screenshot", Intent::Screenshot(Shot::Full)),
    ("Take a screenshot.", Intent::Screenshot(Shot::Full)),
    ("take a screen shot", Intent::Screenshot(Shot::Full)),
    (
        "take a screenshot of the whole screen",
        Intent::Screenshot(Shot::Full),
    ),
    ("screenshot of an area", Intent::Screenshot(Shot::Region)),
    (
        "take a screenshot of a region",
        Intent::Screenshot(Shot::Region),
    ),
    ("region screenshot", Intent::Screenshot(Shot::Region)),
    ("screenshot of the window", Intent::Screenshot(Shot::Window)),
    (
        "take a screenshot of this window",
        Intent::Screenshot(Shot::Window),
    ),
    // power
    ("viloläge", Intent::Suspend),
    ("sätt datorn i viloläge", Intent::Suspend),
    ("gå i vila", Intent::Suspend),
    ("suspend", Intent::Suspend),
    ("go to sleep", Intent::Suspend),
    ("put the computer to sleep", Intent::Suspend),
    ("starta om datorn", Intent::Reboot),
    ("Starta om datorn.", Intent::Reboot),
    ("starta om", Intent::Reboot),
    ("omstart", Intent::Reboot),
    ("restart", Intent::Reboot),
    ("reboot the computer", Intent::Reboot),
    ("restart the computer", Intent::Reboot),
    ("stäng av datorn", Intent::Shutdown),
    ("Stäng av datorn.", Intent::Shutdown),
    ("stäng ner datorn", Intent::Shutdown),
    ("kan du stänga av datorn", Intent::Shutdown),
    ("shut down", Intent::Shutdown),
    ("shut down the computer", Intent::Shutdown),
    ("Shut down the computer.", Intent::Shutdown),
    ("power off", Intent::Shutdown),
    ("turn off the computer", Intent::Shutdown),
    ("logga ut", Intent::Logout),
    ("Logga ut.", Intent::Logout),
    ("log out", Intent::Logout),
    ("sign out", Intent::Logout),
];

/// KB-Whisper often translates English speech into Swedish; what it wrote
/// for the English commands must still mean the same thing.
const TRANSLATED: &[(&str, Intent)] = &[
    ("Öppna terminalen.", Intent::Terminal),
    ("Öppna en terminal.", Intent::Terminal),
    ("Öppna webbläsaren.", Intent::Browser),
    ("Gå till arbetsyta tre.", Intent::Workspace(3)),
    ("Gå till arbetsytan 3.", Intent::Workspace(3)),
    ("Gå till arbetsplats tre.", Intent::Workspace(3)),
    ("Byt till skrivbord två.", Intent::Workspace(2)),
    (
        "Flytta fönstret till arbetsyta två.",
        Intent::MoveToWorkspace(2),
    ),
    ("Nästa arbetsyta.", Intent::NextWorkspace),
    ("Lås skärmen.", Intent::Lock),
    ("Ta en skärmdump.", Intent::Screenshot(Shot::Full)),
    ("Ta en skärmbild.", Intent::Screenshot(Shot::Full)),
    ("Volym upp.", Intent::VolumeUp),
    ("Höj volymen.", Intent::VolumeUp),
    ("Sänk volymen.", Intent::VolumeDown),
    ("Stäng fönstret.", Intent::CloseWindow),
    ("Slå på nattljuset.", Intent::NightLight(Switch::On)),
    ("Stör inte.", Intent::Dnd(Switch::Toggle)),
    ("Stäng av datorn.", Intent::Shutdown),
    ("Starta om datorn.", Intent::Reboot),
    ("Nästa låt.", Intent::NextTrack),
    ("Helskärm.", Intent::Fullscreen),
    ("Översikt.", Intent::Overview),
    ("Logga ut.", Intent::Logout),
    // What KB-Whisper Small wrote for Piper's English voices.
    ("Öppen terminal.", Intent::Terminal),
    ("openterminal.", Intent::Terminal),
    ("Lansera webbläsaren.", Intent::Browser),
    ("Öppna lanseringen.", Intent::Launcher),
    ("Vänd på nattljuset.", Intent::NightLight(Switch::On)),
    ("Stäng av nattlampan,", Intent::NightLight(Switch::Off)),
    ("Möt ljudet.", Intent::Mute),
    ("Möt mikrofonen.", Intent::MicMute),
    (
        "Sänd det här fönstret till arbetsplats 6.",
        Intent::MoveToWorkspace(6),
    ),
    (
        "Sänd detta fönster till arbetsplats 6.",
        Intent::MoveToWorkspace(6),
    ),
    ("Tidigare låt,", Intent::PrevTrack),
    ("Sätt datorn i sömn.", Intent::Suspend),
    ("Switch till Monocle.", Intent::Layout("monocle")),
    ("Stäng fönstren.", Intent::CloseWindow),
    ("Lås skärmarna.", Intent::Lock),
    ("Ny note.", Intent::QuickNoteUi),
    ("Volym down.", Intent::VolumeDown),
    ("Windows jump.", Intent::WindowJump),
];

/// What Whisper wrote for unclear Swedish (Piper voices on othala); the
/// near-miss pass still finds the command.
const MISHEARD: &[(&str, Intent)] = &[
    ("Stada webbläsaren.", Intent::Browser),
    ("Gå till arbets-sita 3.", Intent::Workspace(3)),
    ("Bit till arbetsyta 2.", Intent::Workspace(2)),
    (
        "Flytta fönstret till arbetsyta, fira.",
        Intent::MoveToWorkspace(4),
    ),
    ("Föregående arbets-yta.", Intent::PrevWorkspace),
    ("Gör fönstret flitande!", Intent::Float),
    ("Bit till Monokel.", Intent::Layout("monocle")),
    ("Byt till Monuckel.", Intent::Layout("monocle")),
    ("Nästa lajut.", Intent::NextLayout),
    ("Lås stärmen.", Intent::Lock),
    ("Slå på naturljuset,", Intent::NightLight(Switch::On)),
    ("Stäng av natthuset.", Intent::NightLight(Switch::Off)),
    ("Stäng nu av ljudet.", Intent::Mute),
    ("Ta en kärmbild.", Intent::Screenshot(Shot::Full)),
    ("Kärnbild av ett område.", Intent::Screenshot(Shot::Region)),
    ("Stada om datorn.", Intent::Reboot),
    ("Sätt datorn i villoläge.", Intent::Suspend),
];

#[test]
fn near_misses_still_match() {
    for (heard, want) in MISHEARD {
        assert_eq!(p(heard).as_ref(), Some(want), "{heard:?}");
    }
    // ... but not loosely.
    for heard in [
        "stäng av dörren",
        "Hellerm",
        "Stava sten",
        "gå till arbetsyta tjugo",
        "starta om spotify",
        "byt till monopoly spelet",
        "gå till",
        "gå till tull",
        "Klicka fönstret till arbetsyta 6,",
        "Tredje är arbetsplats.",
        "starta igen",
        "Lås armen.",
        "Starta datorn.",
        "start the computer",
        "Bit till Arbets-sida 2.",
    ] {
        assert_eq!(p(heard), None, "{heard:?}");
    }
    assert!(close("stada", "starta"));
    assert!(close("flitande", "flytande"));
    assert!(!close("bit", "lock"));
    assert!(!close("3", "4"));
}

/// Not understood: nothing may run.
const REJECTED: &[&str] = &[
    "",
    "   ",
    "eh",
    "tack",
    "thank you",
    "hej",
    "stäng av",
    "stäng",
    "turn off",
    "turn on",
    "nästa",
    "next",
    "gå till",
    "arbetsyta",
    "workspace",
    "arbetsyta elva",
    "workspace 11",
    "arbetsyta noll",
    "gå till arbetsyta 42",
    "volym 150 procent",
    "flytta fönstret",
    "öppna",
    "open",
    "starta om spotify nu och sen terminalen igen",
    "öppna terminalen och skriv ls i den",
    "jag tycker om att gå till arbetsyta tre ibland",
    "what is the weather like today",
    "vad är klockan",
    "hur mår du",
    "skriv ett mejl till mamma",
    "rm -rf /",
    "sudo reboot now please and also delete everything",
    "lås upp",
    "unlock",
    "spela spotify",
    "play some jazz on spotify",
    "anteckning k",
    "sök x",
    "open 42",
    "öppna 3",
    "kör om",
    "starta igen",
];

#[test]
fn every_case_means_what_it_says() {
    let mut failures = Vec::new();
    for (heard, want) in CASES {
        let got = p(heard);
        if got.as_ref() != Some(want) {
            failures.push(format!("{heard:?}: want {want:?}, got {got:?}"));
        }
        // The same phrase works for an English user too.
        let got = p_en(heard);
        if got.as_ref() != Some(want) {
            failures.push(format!("(en) {heard:?}: want {want:?}, got {got:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases failed:\n{}",
        failures.len(),
        CASES.len(),
        failures.join("\n")
    );
    assert!(CASES.len() >= 300, "a large table: {}", CASES.len());
}

#[test]
fn translated_english_still_matches() {
    for (heard, want) in TRANSLATED {
        assert_eq!(p(heard).as_ref(), Some(want), "{heard:?}");
    }
}

#[test]
fn unclear_speech_runs_nothing() {
    for heard in REJECTED {
        assert_eq!(p(heard), None, "{heard:?} must not be understood");
        assert_eq!(p_en(heard), None, "(en) {heard:?} must not be understood");
    }
}

#[test]
fn every_documented_example_parses_to_its_rule() {
    for c in compiled() {
        for ex in c.rule.examples {
            let got = parse(ex, c.rule.lang)
                .unwrap_or_else(|| panic!("example {ex:?} ({}) is not understood", c.rule.label));
            let want = build(
                c.rule.kind,
                &Caps {
                    n: match got.intent {
                        Intent::Workspace(n)
                        | Intent::MoveToWorkspace(n)
                        | Intent::VolumeSet(n)
                        | Intent::BrightnessSet(n) => Some(n),
                        _ => None,
                    },
                    text: None,
                },
                &[],
                &[],
            );
            match (&got.intent, want) {
                (Intent::Note(_), _) => assert_eq!(c.rule.kind, K::Note, "{ex:?}"),
                (Intent::Search(_), _) => assert_eq!(c.rule.kind, K::Search, "{ex:?}"),
                (Intent::OpenApp(_), _) => assert_eq!(c.rule.kind, K::OpenApp, "{ex:?}"),
                (got, Some(want)) => assert_eq!(*got, want, "example {ex:?}"),
                (got, None) => panic!("{ex:?}: {got:?} for {}", c.rule.label),
            }
            assert_eq!(got.lang, c.rule.lang, "{ex:?} answers in its own language");
        }
    }
}

#[test]
fn feedback_follows_the_spoken_language() {
    let sv = parse("öppna terminalen", Lang::En).unwrap();
    assert_eq!(sv.lang, Lang::Sv);
    assert_eq!(sv.intent.say(sv.lang), "Öppnar terminal");
    let en = parse("open terminal", Lang::Sv).unwrap();
    assert_eq!(en.lang, Lang::En);
    assert_eq!(en.intent.say(en.lang), "Opening terminal");
    // Words both languages share answer in the preferred one.
    assert_eq!(parse("workspace 3", Lang::Sv).unwrap().lang, Lang::Sv);
    assert_eq!(parse("workspace 3", Lang::En).unwrap().lang, Lang::En);
    assert_eq!(
        parse("mute", Lang::Sv).unwrap().intent.say(Lang::Sv),
        "Ljud av"
    );
    assert_eq!(
        Intent::Shutdown.question(Lang::Sv),
        "Stänga av datorn?".to_string()
    );
    assert_eq!(Intent::Workspace(3).say(Lang::En), "Workspace 3");
}

#[test]
fn free_text_keeps_the_original_words() {
    assert_eq!(
        p("anteckning köp mjölk"),
        Some(Intent::Note("Köp mjölk".into()))
    );
    assert_eq!(
        p("Anteckning: köp mjölk och ägg."),
        Some(Intent::Note("Köp mjölk och ägg".into()))
    );
    assert_eq!(
        p("kom ihåg att ringa mamma"),
        Some(Intent::Note("Ringa mamma".into()))
    );
    assert_eq!(
        p("Kom ihåg att ringa Åsa på fredag."),
        Some(Intent::Note("Ringa Åsa på fredag".into()))
    );
    assert_eq!(
        p("skriv upp att handla bröd"),
        Some(Intent::Note("Handla bröd".into()))
    );
    assert_eq!(p("note buy milk"), Some(Intent::Note("Buy milk".into())));
    assert_eq!(
        p("remember to call the dentist"),
        Some(Intent::Note("Call the dentist".into()))
    );
    assert_eq!(
        p("take a note: the meeting is on Friday"),
        Some(Intent::Note("The meeting is on Friday".into()))
    );
    assert_eq!(
        p("sök efter väder i Stockholm"),
        Some(Intent::Search("väder i Stockholm".into()))
    );
    assert_eq!(
        p("sök på nätet efter rust egui"),
        Some(Intent::Search("rust egui".into()))
    );
    assert_eq!(
        p("googla pannkakor"),
        Some(Intent::Search("pannkakor".into()))
    );
    assert_eq!(
        p("search for the weather in London"),
        Some(Intent::Search("the weather in London".into()))
    );
    assert_eq!(p("look up rust"), Some(Intent::Search("rust".into())));
    assert_eq!(p("öppna spotify"), Some(Intent::OpenApp("spotify".into())));
    assert_eq!(p("Öppna Spotify."), Some(Intent::OpenApp("Spotify".into())));
    assert_eq!(
        p("starta Thunderbird"),
        Some(Intent::OpenApp("Thunderbird".into()))
    );
    assert_eq!(
        p("öppna programmet GIMP"),
        Some(Intent::OpenApp("GIMP".into()))
    );
    assert_eq!(
        p("kan du starta visual studio code"),
        Some(Intent::OpenApp("visual studio code".into()))
    );
    assert_eq!(p("open spotify"), Some(Intent::OpenApp("spotify".into())));
    assert_eq!(
        p("launch Signal please"),
        Some(Intent::OpenApp("Signal".into()))
    );
    assert_eq!(p("run steam"), Some(Intent::OpenApp("steam".into())));
    assert_eq!(
        p("start the GIMP"),
        Some(Intent::OpenApp("the GIMP".into()))
    );
    // Free text is never logged; the name hides it.
    assert_eq!(Intent::Note("secret".into()).name(), "note");
    assert_eq!(Intent::OpenApp("x".into()).name(), "open-app");
    assert_eq!(Intent::NightLight(Switch::On).name(), "nightlight on");
    assert_eq!(Intent::CloseWindow.name(), "close-window");
}

#[test]
fn numbers_in_both_languages() {
    let n = |s: &str| {
        let (_, toks) = tokenize(s);
        numbers(&toks, 0, false).first().map(|x| x.0)
    };
    for (word, value) in [
        ("ett", 1),
        ("två", 2),
        ("tre", 3),
        ("fyra", 4),
        ("fem", 5),
        ("sex", 6),
        ("sju", 7),
        ("åtta", 8),
        ("nio", 9),
        ("tio", 10),
        ("one", 1),
        ("two", 2),
        ("three", 3),
        ("four", 4),
        ("five", 5),
        ("six", 6),
        ("seven", 7),
        ("eight", 8),
        ("nine", 9),
        ("ten", 10),
        ("första", 1),
        ("tredje", 3),
        ("tionde", 10),
        ("third", 3),
        ("3:e", 3),
        ("3:an", 3),
        ("7", 7),
        ("tjugofem", 25),
        ("fyrtiotvå", 42),
        ("twenty five", 25),
        ("twenty-five", 25),
        ("femtio", 50),
        ("hundra", 100),
        ("one hundred", 100),
    ] {
        assert_eq!(n(word), Some(value), "{word}");
    }
    assert_eq!(n("katt"), None);
}

#[test]
fn plans_run_programs_never_a_shell() {
    let cases: &[(Intent, &[&str])] = &[
        (Intent::Terminal, &["niructl", "do", "terminal"]),
        (Intent::Browser, &["niructl", "do", "browser"]),
        (Intent::Workspace(3), &["niructl", "do", "tag", "3"]),
        (
            Intent::MoveToWorkspace(2),
            &["niructl", "do", "movetotag", "2"],
        ),
        (Intent::NextWorkspace, &["niructl", "do", "tag_next"]),
        (Intent::PrevWorkspace, &["niructl", "do", "tag_prev"]),
        (Intent::CloseWindow, &["niructl", "do", "close"]),
        (Intent::Fullscreen, &["niructl", "do", "toggle_fullscreen"]),
        (Intent::Float, &["niructl", "do", "toggle_float"]),
        (
            Intent::Layout("monocle"),
            &["niructl", "do", "layout", "monocle"],
        ),
        (Intent::NextLayout, &["niructl", "do", "layout_next"]),
        (Intent::Lock, &["niructl", "do", "lock"]),
        (Intent::Launcher, &["niructl", "do", "launcher"]),
        (Intent::SearchUi, &["niructl", "do", "launcher_search"]),
        (Intent::WindowJump, &["niructl", "do", "window_hints"]),
        (Intent::QuickNoteUi, &["niructl", "do", "quick_note"]),
        (Intent::Overview, &["niructl", "do", "overview"]),
        (
            Intent::NightLight(Switch::On),
            &["niructl", "nightlight", "on"],
        ),
        (Intent::Dnd(Switch::Toggle), &["niructl", "dnd", "toggle"]),
        (Intent::NextMonitor, &["niructl", "do", "focusmon", "1"]),
        (Intent::PrevMonitor, &["niructl", "do", "focusmon", "-1"]),
        (
            Intent::OpenApp("Spotify".into()),
            &[
                "nirulauncher",
                "--apps",
                "--query",
                "Spotify",
                "--launch-best",
            ],
        ),
        (Intent::VolumeUp, &["niructl", "audio", "volume", "up"]),
        (
            Intent::VolumeSet(40),
            &["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "40%"],
        ),
        (
            Intent::Mute,
            &["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "1"],
        ),
        (Intent::MicMute, &["niructl", "audio", "mic-mute", "on"]),
        (
            Intent::BrightnessSet(30),
            &["niructl", "brightness", "set", "30"],
        ),
        (Intent::Pause, &["playerctl", "pause"]),
        (Intent::Screenshot(Shot::Region), &["nirushot", "--region"]),
        (
            Intent::Note("Köp mjölk".into()),
            &["nirulauncher", "--note", "--stdin"],
        ),
        (
            Intent::Search("väder; rm -rf ~".into()),
            &["nirulauncher", "--search", "väder; rm -rf ~"],
        ),
        (Intent::Suspend, &["niructl", "suspend"]),
        (Intent::Reboot, &["niructl", "reboot"]),
        (Intent::Shutdown, &["niructl", "shutdown"]),
        (Intent::Logout, &["niructl", "logout"]),
    ];
    for (intent, argv) in cases {
        let plan = intent.plan();
        assert_eq!(plan.argv, args(argv), "{intent:?}");
        assert!(
            !["sh", "bash", "fish", "zsh"].contains(&plan.argv[0].as_str()),
            "no shell"
        );
        // `shell` and `start` would run anything; `quit` ends the session.
        if plan.argv[..2] == ["niructl".to_string(), "do".to_string()] {
            assert!(!["shell", "start", "quit"].contains(&plan.argv[2].as_str()));
        }
    }
    assert_eq!(
        Intent::Note("Köp mjölk".into()).plan().stdin.as_deref(),
        Some("Köp mjölk")
    );
    for dangerous in [
        Intent::CloseWindow,
        Intent::Suspend,
        Intent::Reboot,
        Intent::Shutdown,
        Intent::Logout,
    ] {
        assert!(dangerous.plan().dangerous, "{dangerous:?} asks first");
    }
    for safe in [
        Intent::Terminal,
        Intent::Lock,
        Intent::Workspace(1),
        Intent::Mute,
    ] {
        assert!(!safe.plan().dangerous, "{safe:?} runs at once");
    }
    assert!(Intent::Screenshot(Shot::Full).plan().hide_overlay);
    assert!(Intent::Screenshot(Shot::Full).plan().delay_ms > 0);
    assert!(Intent::Lock.plan().hide_overlay);
    assert_eq!(
        Intent::Lock.plan().fallback,
        Some(args(&["niructl", "lock", "--manual"]))
    );
}

#[test]
fn every_rule_has_examples_and_every_intent_a_rule() {
    for r in RULES {
        assert!(!r.examples.is_empty(), "{} has examples", r.label);
        assert!(!r.label.is_empty());
    }
    // Both languages cover the same intents.
    for r in RULES {
        assert!(
            RULES.iter().any(|o| o.kind == r.kind && o.lang != r.lang),
            "{:?} exists in both languages",
            r.kind
        );
    }
    assert_eq!(phrase_list().len(), RULES.len());
}

#[test]
fn not_understood_says_what_was_heard() {
    assert_eq!(
        not_understood(Lang::Sv, " vad är klockan "),
        "Förstod inte: vad är klockan"
    );
    assert_eq!(not_understood(Lang::En, ""), "Not understood");
    let long = not_understood(Lang::En, &"word ".repeat(40));
    assert!(long.ends_with('…'));
    assert!(long.chars().count() < 80);
    assert_eq!(guess_lang("vad är klockan", Lang::En), Lang::Sv);
    assert_eq!(guess_lang("what is the time", Lang::Sv), Lang::En);
    assert_eq!(guess_lang("xyz", Lang::En), Lang::En);
}

#[test]
fn prompts_are_short() {
    for lang in [Lang::Sv, Lang::En] {
        assert!(prompt_words(lang).len() < 450);
    }
}
