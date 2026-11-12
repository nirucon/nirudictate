//! Voice commands: a deterministic, bilingual (Swedish and English) matcher
//! from a transcript to one of a fixed set of desktop actions.
//!
//! There is no language model here. The transcript is normalised (lower case,
//! å/ä/ö folded, punctuation and hesitations removed, politeness such as
//! "kan du", "snälla", "please" stripped) and matched against a table of
//! phrase templates. A template must cover the whole utterance, so "stäng av
//! datorn" never matches "stäng av" and a sentence with something extra is
//! not understood rather than half done. When in doubt, nothing runs.
//!
//! Template syntax (words are folded like the input):
//!
//! - `word`            this word
//! - `(a|b c)`         one of the alternatives (each may be several words)
//! - `[a|b c]`         optional: one of the alternatives or nothing
//! - `{n}`             a number: digits, "tre", "trean", "tredje", "three",
//!   "third", "tjugofem", "twenty five", common mishearings ("to", "for")
//! - `{text}`          the rest of the utterance (one word or more)
//!
//! Rules with `{text}` are tried after every rule without it, and each pass
//! tries the preferred language first, so "workspace 3" answers in Swedish
//! for a Swedish user while "open terminal" is English to everyone.
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Sv,
    En,
}

impl Lang {
    pub fn from_code(code: &str) -> Self {
        if code == "en" {
            Lang::En
        } else {
            Lang::Sv
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Lang::Sv => "sv",
            Lang::En => "en",
        }
    }

    fn pick(self, sv: &str, en: &str) -> String {
        match self {
            Lang::Sv => sv.to_string(),
            Lang::En => en.to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switch {
    On,
    Off,
    Toggle,
}

impl Switch {
    fn word(self) -> &'static str {
        match self {
            Switch::On => "on",
            Switch::Off => "off",
            Switch::Toggle => "toggle",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shot {
    Full,
    Region,
    Window,
}

/// What was asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    Terminal,
    Browser,
    Workspace(u32),
    MoveToWorkspace(u32),
    NextWorkspace,
    PrevWorkspace,
    CloseWindow,
    Fullscreen,
    Float,
    Layout(&'static str),
    NextLayout,
    Lock,
    Launcher,
    SearchUi,
    WindowJump,
    QuickNoteUi,
    Overview,
    NightLight(Switch),
    Dnd(Switch),
    NextMonitor,
    PrevMonitor,
    OpenApp(String),
    VolumeUp,
    VolumeDown,
    VolumeSet(u32),
    Mute,
    Unmute,
    MicMute,
    MicUnmute,
    BrightnessUp,
    BrightnessDown,
    BrightnessSet(u32),
    Play,
    Pause,
    NextTrack,
    PrevTrack,
    StopMedia,
    Screenshot(Shot),
    Note(String),
    Search(String),
    Suspend,
    Reboot,
    Shutdown,
    Logout,
}

/// The table's view of an intent: the same list with the arguments left out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Terminal,
    Browser,
    Workspace,
    MoveToWorkspace,
    NextWorkspace,
    PrevWorkspace,
    CloseWindow,
    Fullscreen,
    Float,
    Layout(&'static str),
    NextLayout,
    Lock,
    Launcher,
    SearchUi,
    WindowJump,
    QuickNoteUi,
    Overview,
    NightLight(Switch),
    Dnd(Switch),
    NextMonitor,
    PrevMonitor,
    OpenApp,
    VolumeUp,
    VolumeDown,
    VolumeSet,
    Mute,
    Unmute,
    MicMute,
    MicUnmute,
    BrightnessUp,
    BrightnessDown,
    BrightnessSet,
    Play,
    Pause,
    NextTrack,
    PrevTrack,
    StopMedia,
    Screenshot(Shot),
    Note,
    Search,
    Suspend,
    Reboot,
    Shutdown,
    Logout,
}

/// The result of matching a transcript.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parsed {
    pub intent: Intent,
    /// The language of the phrase that matched; feedback uses it.
    pub lang: Lang,
}

struct Rule {
    lang: Lang,
    kind: Kind,
    /// What the command does, for `nirudictate command list` and the manual.
    label: &'static str,
    patterns: &'static [&'static str],
    /// Real phrases; the tests check that each one gives this rule's intent.
    examples: &'static [&'static str],
}

// Shared word sets. They are macros so `concat!` can build the patterns.
macro_rules! ws_sv {
    () => {
        "(arbetsyta|arbetsytan|arbetsytor|arbetsplats|arbetsplatsen|arbetsyt|arbetsytta|skrivbord|skrivbordet|skrivbordsyta|skrivbordsytan|yta|ytan|tagg|taggen|tag|taggar|workspace|workspacet|workspacen|desktop|desktopen|vy|vyn)"
    };
}
macro_rules! ws_en {
    () => {
        "(workspace|work space|workspaces|desktop|virtual desktop|tag|view)"
    };
}
macro_rules! go_sv {
    () => {
        "[gå till|gå|byt till|byt|växla till|växla|visa|hoppa till|till|öppna]"
    };
}
macro_rules! go_en {
    () => {
        "[go to|go|switch to|switch|change to|show|jump to|to|open|take me to]"
    };
}
macro_rules! win_sv {
    () => {
        "[fönstret|fönster|det här fönstret|detta fönster|den här|det här|det|den|appen|programmet|aktiva fönstret|aktuella fönstret]"
    };
}
macro_rules! win_en {
    () => {
        "[window|it|app|application|current window|active window]"
    };
}
macro_rules! vol_sv {
    () => {
        "(volym|volymen|ljud|ljudet|ljudnivå|ljudnivån|volume)"
    };
}
macro_rules! vol_en {
    () => {
        "(volume|sound|audio)"
    };
}
macro_rules! mic_sv {
    () => {
        "(mikrofon|mikrofonen|mikrofonerna|micken|mick|mic|mikrofonljudet)"
    };
}
macro_rules! mic_en {
    () => {
        "(mic|microphone|mike)"
    };
}
macro_rules! br_sv {
    () => {
        "(ljusstyrka|ljusstyrkan|skärmljus|skärmljuset|skärmens ljusstyrka|ljuset på skärmen|bakgrundsbelysning|bakgrundsbelysningen|brightness)"
    };
}
macro_rules! br_en {
    () => {
        "(brightness|screen brightness|backlight)"
    };
}
macro_rules! nl_sv {
    () => {
        "(nattljus|nattljuset|nattlampa|nattlampan|nattläge|nattläget|blåljusfilter|blåljusfiltret|kvällsljus|kvällsljuset|night light|nightlight)"
    };
}
macro_rules! nl_en {
    () => {
        "(night light|nightlight|night lights|night mode|night shift|blue light filter)"
    };
}
macro_rules! dnd_sv {
    () => {
        "(stör ej|stör inte|stör ej läge|stör ej läget|störej|do not disturb|dnd)"
    };
}
macro_rules! dnd_en {
    () => {
        "(do not disturb|dont disturb|do not disturb mode|dnd)"
    };
}
macro_rules! mon_sv {
    () => {
        "(skärm|skärmen|bildskärm|bildskärmen|monitor|monitorn|display|displayen)"
    };
}
macro_rules! mon_en {
    () => {
        "(monitor|screen|display)"
    };
}
macro_rules! ss_sv {
    () => {
        "(skärmbild|skärmbilden|skärmdump|skärmdumpen|skärmavbild|skärmavbildning|skärmklipp|skärm bild|skärm dump|screenshot|screenshoten|print screen|printscreen)"
    };
}
macro_rules! ss_en {
    () => {
        "(screenshot|screen shot|screenshots|screen capture|print screen|screengrab|screen grab|snapshot)"
    };
}
macro_rules! layout_sv {
    () => {
        "(layout|layouten|layoutet|lajout|lajut|lajaut|lejaut|läge|läget|layoutläge)"
    };
}
macro_rules! layout_en {
    () => {
        "(layout|mode)"
    };
}

const fn rule(
    lang: Lang,
    kind: Kind,
    label: &'static str,
    patterns: &'static [&'static str],
    examples: &'static [&'static str],
) -> Rule {
    Rule {
        lang,
        kind,
        label,
        patterns,
        examples,
    }
}

use Kind as K;
use Lang::{En, Sv};

static RULES: &[Rule] = &[
    // ── programs ─────────────────────────────────────────────────────────
    rule(Sv, K::Terminal, "terminal", &[
        "[öppna|öppen|starta|kör|visa|ge mig|öppna upp|dra igång|lansera] [en|ett|ny|nytt|den] [ny] (terminal|terminalen|terminaler|terminalfönster|terminalfönstret|kommandotolk|kommandotolken|konsol|konsolen|terminal fönster)",
    "(öppenterminal|öppnaterminal|openterminal)",
    ], &["öppna terminalen", "Öppna en terminal.", "starta terminal", "ny terminal", "terminal", "kan du öppna terminalen tack", "öh öppna terminalen"]),
    rule(En, K::Terminal, "terminal", &[
        "[open|start|launch|run|show|give me|bring up|fire up|open up|get me] [new] (terminal|terminal window|console|shell|command prompt|command line)",
    ], &["open terminal", "Open a terminal.", "open the terminal please", "launch a new terminal", "terminal", "could you open a terminal", "new terminal"]),
    rule(Sv, K::Browser, "browser (the NIRUWM browser)", &[
        "[öppna|öppen|starta|kör|visa|öppna upp|dra igång|lansera] [en|ny|den] [ny] (webbläsare|webbläsaren|webbläsarn|webbläsar|webb läsaren|webb läsare|web läsaren|web läsare|webbläsarfönster|browser|browsern|bråsern|brausern|internet|webben|nätet)",
    ], &["öppna webbläsaren", "starta webbläsaren", "webbläsare", "öppna browsern", "öppna internet", "öppna webb läsaren"]),
    rule(En, K::Browser, "browser (the NIRUWM browser)", &[
        "[open|start|launch|run|show|bring up|fire up|open up|get me] [new] [web|internet] (browser|browser window|internet|web)",
    ], &["open browser", "open the browser", "open a web browser", "launch the browser", "browser", "open the internet"]),
    // ── workspaces ───────────────────────────────────────────────────────
    rule(Sv, K::MoveToWorkspace, "move the window to workspace N", &[
        concat!("(flytta|skicka|sänd|lägg|släng) ", win_sv!(), " (till|på|till den|över till) ", ws_sv!(), " [nummer|nr] {n}"),
        concat!("(flytta|skicka|lägg) ", win_sv!(), " (till|på|över till) [den] {n} ", ws_sv!()),
        concat!("(flytta|skicka) ", win_sv!(), " till {n}"),
    ], &["flytta fönstret till arbetsyta tre", "skicka fönstret till arbetsyta 2", "flytta till arbetsytan fyra", "flytta fönstret till tredje arbetsytan", "flytta det här fönstret till skrivbord 5", "flytta fönstret till trean"]),
    rule(En, K::MoveToWorkspace, "move the window to workspace N", &[
        concat!("(move|send|put|throw) ", win_en!(), " (to|onto|over to) ", ws_en!(), " [number|no|nr] {n}"),
        concat!("(move|send|put) ", win_en!(), " (to|onto) {n} ", ws_en!()),
    ], &["move window to workspace three", "move this window to workspace 2", "send it to desktop four", "move to workspace 5", "move the window to the third workspace"]),
    rule(Sv, K::NextWorkspace, "next workspace", &[
        concat!(go_sv!(), " (nästa|nästkommande) ", ws_sv!()),
        concat!(ws_sv!(), " (framåt|höger|till höger)"),
    ], &["nästa arbetsyta", "gå till nästa arbetsyta", "byt till nästa skrivbord"]),
    rule(En, K::NextWorkspace, "next workspace", &[
        concat!(go_en!(), " next ", ws_en!()),
        concat!(ws_en!(), " (right|forward)"),
    ], &["next workspace", "go to the next workspace", "switch to next desktop"]),
    rule(Sv, K::PrevWorkspace, "previous workspace", &[
        concat!(go_sv!(), " [tillbaka till] (föregående|förra|tidigare) ", ws_sv!()),
        concat!(ws_sv!(), " (bakåt|vänster|till vänster)"),
    ], &["föregående arbetsyta", "gå till förra arbetsytan", "gå tillbaka till förra skrivbordet"]),
    rule(En, K::PrevWorkspace, "previous workspace", &[
        concat!(go_en!(), " [back to] (previous|prev) ", ws_en!()),
        concat!(ws_en!(), " (left|back)"),
    ], &["previous workspace", "go to the previous workspace", "go back to previous desktop"]),
    rule(Sv, K::Workspace, "workspace N", &[
        concat!(go_sv!(), " ", ws_sv!(), " [nummer|nr] {n}"),
        concat!(go_sv!(), " [den] {n} ", ws_sv!()),
    ], &["arbetsyta tre", "gå till arbetsyta 3", "byt till arbetsytan två", "visa skrivbord fyra", "gå till tredje arbetsytan", "arbetsyta nummer sju", "gå till arbetsyta 3:an", "växla till workspace fem", "arbetsyta tio"]),
    rule(En, K::Workspace, "workspace N", &[
        concat!(go_en!(), " ", ws_en!(), " [number|no|nr] {n}"),
        concat!(go_en!(), " {n} ", ws_en!()),
    ], &["workspace three", "go to workspace 3", "switch to workspace two", "show desktop four", "go to the second workspace", "workspace number seven", "go to workspace to", "workspace for"]),
    // ── windows ──────────────────────────────────────────────────────────
    rule(Sv, K::CloseWindow, "close the window (asks first)", &[
        "stäng [det här|den här|aktiva|aktuella] (fönstret|fönstren|fönster|appen|programmet|det här fönstret|det|den)",
        "stäng det här",
    ], &["stäng fönstret", "stäng det här fönstret", "stäng appen"]),
    rule(En, K::CloseWindow, "close the window (asks first)", &[
        "close [current|active] (window|app|application|program|it)",
    ], &["close window", "close the window", "close this window", "close the app", "close it"]),
    rule(Sv, K::Fullscreen, "fullscreen on/off", &[
        "[växla|gör|visa|sätt|slå på|aktivera|gå till|avsluta|lämna|stäng av|stäng] [fönstret|det] [i|till|på] (helskärm|fullskärm|helskärmsläge|fullskärmsläge|helskärmsläget|fullskärmsläget|full skärm|hel skärm|fullscreen)",
    ], &["helskärm", "växla helskärm", "gör fönstret till helskärm", "fullskärm", "avsluta helskärm"]),
    rule(En, K::Fullscreen, "fullscreen on/off", &[
        "[toggle|make|go|set|enter|exit|leave|turn on|turn off] [window|it] [to|into] (fullscreen|full screen|fullscreen mode)",
    ], &["fullscreen", "toggle fullscreen", "make it full screen", "go full screen", "exit fullscreen"]),
    rule(Sv, K::Float, "floating window on/off", &[
        "[växla|gör|sätt] [fönstret|det] [till] (flytande|svävande) [fönster]",
        "låt (fönstret|det) (flyta|sväva)",
    ], &["flytande fönster", "gör fönstret flytande", "växla flytande"]),
    rule(En, K::Float, "floating window on/off", &[
        "[toggle|make|set] [window|it] (float|floating) [window]",
        "float [window|it]",
    ], &["float", "toggle floating", "make the window floating", "float this window"]),
    rule(Sv, K::NextLayout, "next layout", &[
        concat!("(nästa|byt|växla|ändra) ", layout_sv!()),
    ], &["nästa layout", "byt layout"]),
    rule(En, K::NextLayout, "next layout", &[
        concat!("(next|change|switch|cycle) ", layout_en!()),
    ], &["next layout", "change layout", "cycle layout"]),
    rule(Sv, K::Layout("tile"), "layout tile", &[
        concat!("[byt|växla|sätt|ändra|använd|gå|switch] [till] ", layout_sv!(), " [till] (tile|tiling|kakel|kakla|rutnät|sida vid sida|staplat|stack|master)"),
        concat!("[byt|växla|sätt|ändra|använd|gå|switch] [till] (tile|tiling|kakel|kakla|rutnät|sida vid sida|staplat|stack|master|kakelläge) [", layout_sv!(), "]"),
    ], &["layout tile", "byt till kakel", "växla till tiling layout", "sida vid sida"]),
    rule(En, K::Layout("tile"), "layout tile", &[
        concat!("[switch to|change to|set|use|go to] [to] ", layout_en!(), " (tile|tiling|tiled|master stack)"),
        concat!("[switch to|change to|set|use|go to] (tile|tiling|tiled|master stack) [", layout_en!(), "]"),
    ], &["tiling layout", "switch to tile layout", "layout tile"]),
    rule(Sv, K::Layout("monocle"), "layout monocle", &[
        concat!("[byt|växla|sätt|ändra|använd|gå|switch] [till] ", layout_sv!(), " [till] (monocle|monokel|monokelläge|ett fönster i taget|ett i taget)"),
        concat!("[byt|växla|sätt|ändra|använd|gå|switch] [till] (monocle|monokel|monokelläge|ett fönster i taget) [", layout_sv!(), "]"),
    ], &["monokel", "byt till monokel", "layout monocle", "ett fönster i taget"]),
    rule(En, K::Layout("monocle"), "layout monocle", &[
        concat!("[switch to|change to|set|use|go to] [to] ", layout_en!(), " (monocle|monocle mode|one window at time)"),
        concat!("[switch to|change to|set|use|go to] (monocle|one window at time) [", layout_en!(), "]"),
    ], &["monocle", "switch to monocle", "monocle layout"]),
    rule(Sv, K::Layout("floating"), "layout floating", &[
        concat!("[byt|växla|sätt|ändra|använd|gå|switch] [till] ", layout_sv!(), " [till] (flytande|floating|fri|fritt)"),
        concat!("[byt|växla|sätt|ändra|använd|gå|switch] [till] (flytande|floating|fri|fritt) ", layout_sv!()),
    ], &["flytande layout", "byt till flytande läge", "layout floating"]),
    rule(En, K::Layout("floating"), "layout floating", &[
        concat!("[switch to|change to|set|use|go to] [to] ", layout_en!(), " (floating|float|free)"),
        concat!("[switch to|change to|set|use|go to] (floating|float|free) ", layout_en!()),
    ], &["floating layout", "switch to floating layout"]),
    rule(Sv, K::Layout("centered"), "layout centered", &[
        concat!("[byt|växla|sätt|ändra|använd|gå|switch] [till] ", layout_sv!(), " [till] (centrerad|centrerat|centered|center|mitten)"),
        concat!("[byt|växla|sätt|ändra|använd|gå|switch] [till] (centrerad|centrerat|centered|centrerade) ", layout_sv!()),
    ], &["centrerad layout", "layout centrerad"]),
    rule(En, K::Layout("centered"), "layout centered", &[
        concat!("[switch to|change to|set|use|go to] [to] ", layout_en!(), " (centered|centred|center|centre)"),
        concat!("[switch to|change to|set|use|go to] (centered|centred|center|centre) ", layout_en!()),
    ], &["centered layout", "layout center"]),
    rule(Sv, K::Layout("fibonacci"), "layout fibonacci", &[
        concat!("[byt|växla|sätt|ändra|använd|gå|switch] [till] ", layout_sv!(), " [till] (fibonacci|spiral|spiralen)"),
        concat!("[byt|växla|sätt|ändra|använd|gå|switch] [till] (fibonacci|spiral) [", layout_sv!(), "]"),
    ], &["fibonacci", "layout spiral", "byt till fibonacci"]),
    rule(En, K::Layout("fibonacci"), "layout fibonacci", &[
        concat!("[switch to|change to|set|use|go to] [to] ", layout_en!(), " (fibonacci|spiral)"),
        concat!("[switch to|change to|set|use|go to] (fibonacci|spiral) [", layout_en!(), "]"),
    ], &["spiral layout", "switch to fibonacci"]),
    rule(Sv, K::Layout("scroll"), "layout scroll", &[
        concat!("[byt|växla|sätt|ändra|använd|gå|switch] [till] ", layout_sv!(), " [till] (scroll|scrolla|scrollande|rullande|rulla)"),
        concat!("[byt|växla|sätt|ändra|använd|gå|switch] [till] (scroll|scrollande|rullande) ", layout_sv!()),
    ], &["layout scroll", "rullande layout"]),
    rule(En, K::Layout("scroll"), "layout scroll", &[
        concat!("[switch to|change to|set|use|go to] [to] ", layout_en!(), " (scroll|scrolling)"),
        concat!("[switch to|change to|set|use|go to] (scroll|scrolling) ", layout_en!()),
    ], &["scrolling layout", "layout scroll"]),
    // ── session and tools ────────────────────────────────────────────────
    rule(Sv, K::Lock, "lock the screen", &[
        "(lås|låsa) [skärmen|skärmarna|skärm|datorn|dator|den|allt|mig|låsskärmen]",
        "(låsskärm|låsskärmen|lås skärm|låst skärm|skärmlås)",
    ], &["lås skärmen", "Lås skärmen.", "lås datorn", "lås", "låsskärm", "kan du låsa skärmen"]),
    rule(En, K::Lock, "lock the screen", &[
        "lock [screen|computer|pc|session|it|up|desktop|laptop|machine]",
        "(lock screen|screen lock|lockscreen)",
    ], &["lock screen", "lock the screen", "lock my computer", "lock", "please lock the screen"]),
    rule(Sv, K::Launcher, "launcher", &[
        "[öppna|visa|starta] (lanseraren|lanseringen|startaren|startmeny|startmenyn|programmeny|programmenyn|appmeny|appmenyn|appstartare|appstartaren|programstartare|programstartaren|launcher|launchern|appar|apparna|program|programmen|alla appar|alla program|applikationer|applikationerna|menyn)",
    ], &["öppna startmenyn", "visa appar", "programmenyn", "öppna launchern"]),
    rule(En, K::Launcher, "launcher", &[
        "[open|show|bring up] (launcher|app launcher|application launcher|apps|applications|app menu|start menu|menu|programs|all apps|app list)",
    ], &["open the launcher", "show apps", "app launcher", "open start menu"]),
    rule(Sv, K::SearchUi, "web search box", &[
        "[öppna|visa] (sök|sökning|sökningen|söka|sökrutan|sökfältet|webbsök|webbsökning)",
    ], &["sök", "öppna sökning", "webbsök"]),
    rule(En, K::SearchUi, "web search box", &[
        "[open|show] (search|web search|search box|search bar)",
    ], &["search", "open search", "web search"]),
    rule(Sv, K::WindowJump, "window jump (letters on windows)", &[
        "(fönsterhopp|windowsjump|windows jump|window jump|hoppa till fönster|hoppa till ett fönster|hoppa mellan fönster|välj fönster|välj ett fönster|fönsterbokstäver|visa fönsterbokstäver|fönsterväljare|fönsterväljaren)",
    ], &["fönsterhopp", "hoppa till fönster", "välj fönster"]),
    rule(En, K::WindowJump, "window jump (letters on windows)", &[
        "(window jump|jump to window|jump to window hints|window hints|hints|pick window|choose window|pick window hint)",
    ], &["window jump", "jump to a window", "window hints"]),
    rule(Sv, K::QuickNoteUi, "quick note window", &[
        "[ny|en|ny snabb|skriv en|skriv|öppna|gör en|ta en] (anteckning|snabbanteckning|notering|anteckningar)",
        "ny (note|not|notis)",
        "(anteckna|notera)",
    ], &["ny anteckning", "anteckning", "snabbanteckning", "skriv en anteckning"]),
    rule(En, K::QuickNoteUi, "quick note window", &[
        "[new|take|open|write|make|add|quick] (note|quick note|notes)",
    ], &["new note", "take a note", "quick note", "note"]),
    rule(Sv, K::Overview, "overview of the workspaces", &[
        "[öppna|visa] (översikt|översikten|översiktsvy|översiktsvyn|översiktsläge)",
        "(visa|öppna) [alla] (arbetsytor|arbetsytorna|skrivbord|skrivborden|fönster|fönstren)",
        "alla (arbetsytor|arbetsytorna|skrivborden|fönstren)",
    ], &["översikt", "visa översikten", "visa alla arbetsytor"]),
    rule(En, K::Overview, "overview of the workspaces", &[
        "[open|show] (overview|workspace overview|expose)",
        "[open|show] all (workspaces|windows|desktops)",
        "show (workspaces|desktops)",
    ], &["overview", "show all workspaces", "show the overview"]),
    rule(Sv, K::NightLight(Switch::On), "night light on", &[
        concat!("(slå på|sätt på|vänd på|tänd|aktivera|starta|sätt igång|på med|slå igång|koppla på) ", nl_sv!()),
        concat!(nl_sv!(), " (på|igång)"),
        concat!("(sätt|slå) ", nl_sv!(), " på"),
    ], &["slå på nattljus", "nattljus på", "sätt på nattljuset", "aktivera nattläge"]),
    rule(En, K::NightLight(Switch::On), "night light on", &[
        concat!("(turn on|switch on|enable|activate|start) ", nl_en!()),
        concat!(nl_en!(), " on"),
        concat!("(turn|switch|put) ", nl_en!(), " on"),
    ], &["turn on night light", "night light on", "enable night mode", "turn the night light on"]),
    rule(Sv, K::NightLight(Switch::Off), "night light off", &[
        concat!("(stäng av|slå av|vänd av|tänd av|släck|avaktivera|stoppa|ta bort|av med|inaktivera|koppla av|koppla bort) ", nl_sv!()),
        concat!(nl_sv!(), " av"),
        concat!("(sätt|slå|stäng) ", nl_sv!(), " av"),
    ], &["stäng av nattljuset", "nattljus av", "släck nattljuset", "ta bort nattläget"]),
    rule(En, K::NightLight(Switch::Off), "night light off", &[
        concat!("(turn off|switch off|disable|deactivate|stop) ", nl_en!()),
        concat!(nl_en!(), " off"),
        concat!("(turn|switch|put) ", nl_en!(), " off"),
    ], &["turn off night light", "night light off", "disable the night light"]),
    rule(Sv, K::NightLight(Switch::Toggle), "night light on/off", &[
        concat!("[växla|byt] ", nl_sv!()),
    ], &["nattljus", "växla nattljus"]),
    rule(En, K::NightLight(Switch::Toggle), "night light on/off", &[
        concat!("[toggle|switch] ", nl_en!()),
    ], &["night light", "toggle night light"]),
    rule(Sv, K::Dnd(Switch::On), "do not disturb on", &[
        concat!("(slå på|sätt på|vänd på|aktivera|starta|på med|koppla på) ", dnd_sv!()),
        concat!(dnd_sv!(), " (på|igång)"),
        concat!("(sätt|slå) ", dnd_sv!(), " på"),
    ], &["slå på stör ej", "stör ej på", "aktivera stör ej"]),
    rule(En, K::Dnd(Switch::On), "do not disturb on", &[
        concat!("(turn on|switch on|enable|activate|start) ", dnd_en!()),
        concat!(dnd_en!(), " on"),
        concat!("(turn|switch|put) ", dnd_en!(), " on"),
    ], &["turn on do not disturb", "do not disturb on", "enable do-not-disturb", "don't disturb on"]),
    rule(Sv, K::Dnd(Switch::Off), "do not disturb off", &[
        concat!("(stäng av|slå av|vänd av|vänd bort|avaktivera|stoppa|ta bort|av med|inaktivera|koppla av) ", dnd_sv!()),
        concat!(dnd_sv!(), " av"),
        concat!("(sätt|slå|stäng) ", dnd_sv!(), " av"),
    ], &["stäng av stör ej", "stör ej av", "ta bort stör ej"]),
    rule(En, K::Dnd(Switch::Off), "do not disturb off", &[
        concat!("(turn off|switch off|disable|deactivate|stop) ", dnd_en!()),
        concat!(dnd_en!(), " off"),
        concat!("(turn|switch|put) ", dnd_en!(), " off"),
    ], &["turn off do not disturb", "do not disturb off", "disable dnd"]),
    rule(Sv, K::Dnd(Switch::Toggle), "do not disturb on/off", &[
        concat!("[växla] ", dnd_sv!()),
    ], &["stör ej", "växla stör ej"]),
    rule(En, K::Dnd(Switch::Toggle), "do not disturb on/off", &[
        concat!("[toggle] ", dnd_en!()),
    ], &["do not disturb", "toggle do not disturb"]),
    rule(Sv, K::NextMonitor, "next screen", &[
        concat!("[gå till|byt till|växla till|fokusera|gå|byt] (nästa|andra|den andra|nästa skärm) ", mon_sv!()),
    ], &["nästa skärm", "byt till nästa skärm", "gå till andra skärmen", "fokusera nästa bildskärm"]),
    rule(En, K::NextMonitor, "next screen", &[
        concat!("[go to|switch to|focus|move to|change to] (next|other) ", mon_en!()),
    ], &["next monitor", "switch to the other screen", "focus next display"]),
    rule(Sv, K::PrevMonitor, "previous screen", &[
        concat!("[gå till|byt till|växla till|fokusera|gå|byt] (föregående|förra) ", mon_sv!()),
    ], &["föregående skärm", "gå till förra skärmen"]),
    rule(En, K::PrevMonitor, "previous screen", &[
        concat!("[go to|switch to|focus|move to|change to] (previous|prev) ", mon_en!()),
    ], &["previous monitor", "focus the previous screen"]),
    // ── sound and screen ─────────────────────────────────────────────────
    rule(Sv, K::VolumeSet, "volume N %", &[
        concat!("[sätt|ställ|ändra|höj|sänk|skruva] ", vol_sv!(), " (till|på) {n} [procent|%]"),
        concat!(vol_sv!(), " [till|på] {n} [procent|%]"),
    ], &["volym 50 procent", "sätt volymen till 30 %", "volymen på femtio", "ställ volymen på tjugofem procent", "sänk volymen till 20"]),
    rule(En, K::VolumeSet, "volume N %", &[
        concat!("[set|change|put|turn] ", vol_en!(), " (to|at) {n} [percent|%]"),
        concat!(vol_en!(), " [at] {n} [percent|%]"),
    ], &["volume 50 percent", "set volume to 30%", "set the volume to twenty five percent", "volume at 40"]),
    rule(Sv, K::VolumeUp, "volume up", &[
        concat!("(höj|öka|skruva upp|dra upp|vrid upp|höja) [på] ", vol_sv!(), " [lite|mer|lite till]"),
        concat!(vol_sv!(), " (upp|högre|uppåt|up|app)"),
        "(högre|lite högre|högre ljud|mer ljud|högre volym|mer volym|skruva upp|starkare)",
    ], &["höj volymen", "volym upp", "högre", "mer ljud", "höj ljudet lite", "skruva upp volymen"]),
    rule(En, K::VolumeUp, "volume up", &[
        concat!("(turn up|increase|raise|crank up|pump up) [the] ", vol_en!()),
        concat!(vol_en!(), " (up|louder|higher)"),
        "turn (it|volume|sound|audio) up",
        "(louder|more volume|turn up|bit louder|little louder)",
    ], &["volume up", "turn up the volume", "louder", "increase the volume", "turn it up"]),
    rule(Sv, K::VolumeDown, "volume down", &[
        concat!("(sänk|minska|skruva ner|skruva ned|dra ner|dra ned|vrid ner|sänka) [på] ", vol_sv!(), " [lite|mer|lite till]"),
        concat!(vol_sv!(), " (ner|ned|lägre|nedåt|down)"),
        "(lägre|tystare|mindre ljud|lägre ljud|lite lägre|lite tystare|lägre volym|mindre volym|skruva ner|svagare)",
    ], &["sänk volymen", "volym ner", "tystare", "lägre", "sänk ljudet lite"]),
    rule(En, K::VolumeDown, "volume down", &[
        concat!("(turn down|decrease|lower|reduce) [the] ", vol_en!()),
        concat!(vol_en!(), " (down|lower|quieter)"),
        "turn (it|volume|sound|audio) down",
        "(quieter|softer|less volume|turn down|bit quieter|little quieter)",
    ], &["volume down", "turn down the volume", "quieter", "lower the volume", "turn it down"]),
    rule(Sv, K::MicMute, "microphone off", &[
        concat!("(stäng av|slå av|tysta|muta|mute|mjuta|möt|stäng) ", mic_sv!()),
        concat!(mic_sv!(), " (av|tyst)"),
        concat!("(sätt|slå) ", mic_sv!(), " av"),
    ], &["stäng av mikrofonen", "mikrofon av", "muta mikrofonen", "tysta micken"]),
    rule(En, K::MicMute, "microphone off", &[
        concat!("(mute|turn off|switch off|disable|silence) ", mic_en!()),
        concat!(mic_en!(), " (off|mute|muted)"),
    ], &["mute the microphone", "mute mic", "microphone off", "turn off the mic"]),
    rule(Sv, K::MicUnmute, "microphone on", &[
        concat!("(slå på|sätt på|aktivera|unmuta|avmuta|unmute|starta) ", mic_sv!()),
        concat!(mic_sv!(), " på"),
        concat!("(sätt|slå) ", mic_sv!(), " på"),
    ], &["slå på mikrofonen", "mikrofon på", "avmuta mikrofonen"]),
    rule(En, K::MicUnmute, "microphone on", &[
        concat!("(unmute|turn on|switch on|enable) ", mic_en!()),
        concat!(mic_en!(), " (on|unmute|unmuted)"),
    ], &["unmute the microphone", "unmute mic", "microphone on", "turn on the mic"]),
    rule(Sv, K::Mute, "sound off", &[
        concat!("(stäng av|slå av|tysta|muta|mute|mjuta|möt|stäng) ", vol_sv!()),
        concat!(vol_sv!(), " (av|tyst)"),
        concat!("(sätt|slå) ", vol_sv!(), " av"),
        "(mute|muta|mjuta|tyst|ljudlöst)",
    ], &["stäng av ljudet", "ljud av", "tysta ljudet", "mute", "muta ljudet"]),
    rule(En, K::Mute, "sound off", &[
        concat!("(mute|turn off|switch off|silence|kill) ", vol_en!()),
        concat!(vol_en!(), " (off|mute|muted)"),
        concat!("(turn|switch) ", vol_en!(), " off"),
        "(mute|silence|mute everything)",
    ], &["mute", "mute the sound", "sound off", "turn off the sound"]),
    rule(Sv, K::Unmute, "sound on", &[
        concat!("(slå på|sätt på|aktivera|unmuta|avmuta|unmute|starta) ", vol_sv!()),
        concat!(vol_sv!(), " på"),
        concat!("(sätt|slå) ", vol_sv!(), " på"),
        "(unmuta|avmuta|unmute)",
    ], &["slå på ljudet", "ljud på", "sätt på ljudet", "unmute"]),
    rule(En, K::Unmute, "sound on", &[
        concat!("(unmute|turn on|switch on|enable) ", vol_en!()),
        concat!(vol_en!(), " (on|unmute|unmuted|back on)"),
        concat!("(turn|switch|put) ", vol_en!(), " [back] on"),
        "unmute",
    ], &["unmute", "unmute the sound", "sound on", "turn the sound back on"]),
    rule(Sv, K::BrightnessSet, "brightness N %", &[
        concat!("[sätt|ställ|ändra|höj|sänk] ", br_sv!(), " (till|på) {n} [procent|%]"),
        concat!(br_sv!(), " [till|på] {n} [procent|%]"),
    ], &["ljusstyrka 50 procent", "sätt ljusstyrkan till 30 %", "ljusstyrkan på sjuttio"]),
    rule(En, K::BrightnessSet, "brightness N %", &[
        concat!("[set|change|put|turn] ", br_en!(), " (to|at) {n} [percent|%]"),
        concat!(br_en!(), " [at] {n} [percent|%]"),
    ], &["brightness 50 percent", "set brightness to 30%", "set the screen brightness to seventy percent"]),
    rule(Sv, K::BrightnessUp, "brighter", &[
        concat!("(höj|öka|skruva upp|dra upp|höja) [på] ", br_sv!(), " [lite|mer]"),
        concat!(br_sv!(), " (upp|högre|uppåt)"),
        "(ljusare|ljusare skärm|gör skärmen ljusare|skärmen ljusare|lite ljusare|mer ljus)",
    ], &["ljusare", "höj ljusstyrkan", "ljusstyrka upp", "gör skärmen ljusare"]),
    rule(En, K::BrightnessUp, "brighter", &[
        concat!("(turn up|increase|raise) [the] ", br_en!()),
        concat!(br_en!(), " (up|higher)"),
        concat!("turn ", br_en!(), " up"),
        "(brighter|make screen brighter|screen brighter|bit brighter|little brighter)",
    ], &["brighter", "brightness up", "increase the brightness", "make the screen brighter"]),
    rule(Sv, K::BrightnessDown, "dimmer", &[
        concat!("(sänk|minska|skruva ner|dra ner|sänka|dimma) [på] ", br_sv!(), " [lite|mer]"),
        concat!(br_sv!(), " (ner|ned|lägre|nedåt)"),
        "(mörkare|mörkare skärm|gör skärmen mörkare|skärmen mörkare|lite mörkare|dimma|dimma skärmen|mindre ljus)",
    ], &["mörkare", "sänk ljusstyrkan", "ljusstyrka ner", "dimma skärmen"]),
    rule(En, K::BrightnessDown, "dimmer", &[
        concat!("(turn down|decrease|lower|reduce|dim) [the] ", br_en!()),
        concat!(br_en!(), " (down|lower)"),
        concat!("turn ", br_en!(), " down"),
        "(dimmer|darker|dim screen|dim|make screen darker|make screen dimmer|screen darker|bit darker|little darker)",
    ], &["dimmer", "brightness down", "dim the screen", "make the screen darker"]),
    // ── media ────────────────────────────────────────────────────────────
    rule(Sv, K::NextTrack, "next track", &[
        "[spela|gå till|byt till] (nästa) (låt|låten|spår|spåret|sång|sången)",
        "(hoppa över|skippa|skip) [låten|den här låten|spåret|den här]",
        "byt låt",
    ], &["nästa låt", "hoppa över låten", "skippa", "byt låt", "spela nästa låt"]),
    rule(En, K::NextTrack, "next track", &[
        "[play|go to] next (song|track|tune)",
        "skip [song|track|this song|this track|it]",
    ], &["next song", "next track", "skip", "skip this song", "play the next track"]),
    rule(Sv, K::PrevTrack, "previous track", &[
        "[spela|gå till|byt till|gå tillbaka till] (föregående|förra|tidigare) (låt|låten|spår|spåret|sång|sången)",
        "(tillbaka en låt|en låt tillbaka|backa en låt)",
    ], &["föregående låt", "spela förra låten", "tillbaka en låt"]),
    rule(En, K::PrevTrack, "previous track", &[
        "[play|go to|go back to] (previous|prev|last) (song|track|tune)",
        "(back one song|go back one song|back one track)",
    ], &["previous song", "play the previous track", "last song"]),
    rule(Sv, K::Pause, "pause", &[
        "(pausa|paus|pausa musiken|pausa låten|pausa uppspelningen|pausa videon|pausa spelaren)",
    ], &["pausa", "paus", "pausa musiken"]),
    rule(En, K::Pause, "pause", &[
        "(pause|pause music|pause song|pause playback|pause video|pause player)",
    ], &["pause", "pause the music"]),
    rule(Sv, K::Play, "play", &[
        "(spela|spela upp|spela musik|spela musiken|starta musiken|fortsätt|fortsätt spela|fortsätt spela musiken|återuppta|fortsätt musiken|play)",
    ], &["spela", "spela musik", "fortsätt spela"]),
    rule(En, K::Play, "play", &[
        "(play|play music|resume|resume music|resume playback|continue playing|unpause|start music)",
    ], &["play", "play music", "resume"]),
    rule(Sv, K::StopMedia, "stop playback", &[
        "(stoppa|stoppa musiken|stanna musiken|stäng av musiken|sluta spela|stoppa uppspelningen)",
    ], &["stoppa musiken", "sluta spela"]),
    rule(En, K::StopMedia, "stop playback", &[
        "(stop music|stop playback|stop playing|stop song|stop)",
    ], &["stop the music", "stop playing"]),
    rule(Sv, K::Screenshot(Shot::Region), "screenshot of an area", &[
        concat!("[ta|ta en|gör en|tar|ta ett] ", ss_sv!(), " (av|på|över) (ett område|område|området|en del|en del av skärmen|del av skärmen|ett urval|urval|markering|en markering|region|en region|ett utsnitt|utsnitt)"),
        "(områdesskärmbild|områdesbild|klipp ut ett område|skärmklipp av område)",
    ], &["skärmbild av ett område", "ta en skärmbild av en del av skärmen", "områdesskärmbild"]),
    rule(En, K::Screenshot(Shot::Region), "screenshot of an area", &[
        concat!("[take|grab|make|capture] ", ss_en!(), " (of) (region|area|selection|part of screen|part|portion|section)"),
        concat!("(region|area|partial|selection) ", ss_en!()),
    ], &["screenshot of an area", "take a screenshot of a region", "region screenshot"]),
    rule(Sv, K::Screenshot(Shot::Window), "screenshot of the window", &[
        concat!("[ta|ta en|gör en|tar|ta ett] ", ss_sv!(), " (av|på) (fönstret|fönster|ett fönster|det här fönstret|aktiva fönstret|appen)"),
        "(fönsterskärmbild|fönsterbild|skärmbild fönster)",
    ], &["skärmbild av fönstret", "ta en skärmdump av det här fönstret"]),
    rule(En, K::Screenshot(Shot::Window), "screenshot of the window", &[
        concat!("[take|grab|make|capture] ", ss_en!(), " (of) (window|this window|active window|app|current window)"),
        concat!("window ", ss_en!()),
    ], &["screenshot of the window", "take a screenshot of this window", "window screenshot"]),
    rule(Sv, K::Screenshot(Shot::Full), "screenshot", &[
        concat!("[ta|ta en|gör en|tar|ta ett] ", ss_sv!(), " [av|på] [hela skärmen|skärmen|hela skrivbordet|skrivbordet|allt|allting]"),
        "(fota|fotografera|ta en bild av|ta bild av) [hela] (skärmen|skrivbordet)",
    ], &["skärmbild", "ta en skärmbild", "ta en skärmdump av hela skärmen", "fota skärmen", "screenshot"]),
    rule(En, K::Screenshot(Shot::Full), "screenshot", &[
        concat!("[take|grab|make|capture] ", ss_en!(), " [of] [whole screen|full screen|entire screen|everything|screen|desktop]"),
        concat!("(full|full screen|whole screen) ", ss_en!()),
        "(take picture of|capture|grab) (screen|whole screen|desktop)",
    ], &["screenshot", "take a screenshot", "take a screenshot of the whole screen", "capture the screen"]),
    // ── power (always asks first) ────────────────────────────────────────
    rule(Sv, K::Suspend, "suspend (asks first)", &[
        "(vila|viloläge|viloläget|sov|somna|suspend|suspendera)",
        "(sätt|försätt|lägg|ställ) [datorn|den] i (viloläge|vila|sömn)",
        "(gå i|gå till) (viloläge|vila)",
        "(datorn|dator) (vila|sov)",
    ], &["viloläge", "sätt datorn i viloläge", "gå i vila", "vila"]),
    rule(En, K::Suspend, "suspend (asks first)", &[
        "(suspend|sleep|go to sleep|sleep mode|suspend computer|computer sleep|standby)",
        "put [computer|it|pc|laptop] to sleep",
    ], &["suspend", "sleep", "go to sleep", "put the computer to sleep"]),
    rule(Sv, K::Reboot, "restart the computer (asks first)", &[
        "(starta om|omstart|reboota|rebuta|reboot|restart|boota om|starta om igen) [datorn|dator|systemet|allt|maskinen]",
    ], &["starta om datorn", "starta om", "omstart", "reboota"]),
    rule(En, K::Reboot, "restart the computer (asks first)", &[
        "(reboot|restart|reboot computer|restart computer|restart system|reboot system|restart pc|reboot pc|restart machine)",
    ], &["restart", "reboot the computer", "restart the computer"]),
    rule(Sv, K::Shutdown, "shut down (asks first)", &[
        "(stäng av|stäng ner|stäng ned|slå av|stäng) (datorn|dator|systemet|allt|maskinen|hela datorn)",
        "(stäng ner|stäng ned|shutdown|shut down|power off|stänga av datorn)",
    ], &["stäng av datorn", "stäng ner datorn", "stäng ner", "slå av datorn"]),
    rule(En, K::Shutdown, "shut down (asks first)", &[
        "(shut down|shutdown|power off|power down) [computer|pc|system|machine|laptop|everything]",
        "(turn off|switch off|shut down|power off|power down) (computer|pc|system|machine|laptop|everything)",
        "(shut|turn) (computer|pc|system|machine) (down|off)",
    ], &["shut down", "shut down the computer", "power off", "turn off the computer", "shutdown"]),
    rule(Sv, K::Logout, "log out (asks first)", &[
        "(logga ut|logga av|utloggning|log out|logout) [mig|från sessionen|ur sessionen]",
    ], &["logga ut", "logga ut mig"]),
    rule(En, K::Logout, "log out (asks first)", &[
        "(log out|logout|log off|sign out|log me out|sign me out)",
    ], &["log out", "sign out", "log me out"]),
    // ── with free text (tried after everything above) ────────────────────
    rule(Sv, K::Note, "note TEXT (saved to the inbox)", &[
        "(anteckning|anteckna|notera|notering|ny anteckning|kom ihåg|skriv upp|skriv ner|spara anteckning|spara en anteckning|anteckna att|notera att) [att] {text}",
    ], &["anteckning köp mjölk", "kom ihåg att ringa mamma", "anteckna möte på fredag", "skriv upp att handla bröd"]),
    rule(En, K::Note, "note TEXT (saved to the inbox)", &[
        "(note|take note|make note|new note|remember|write down|jot down|add note|save note) [to|that] {text}",
    ], &["note buy milk", "remember to call mom", "take a note meeting on friday", "write down buy bread"]),
    rule(Sv, K::Search, "search the web for TEXT", &[
        "(sök|sök efter|sök på|söka efter|leta efter|leta upp|googla|googla på|googla efter|webbsök|webbsök efter|sök på nätet efter|sök på webben efter) {text}",
    ], &["sök efter väder stockholm", "googla rust egui", "sök recept på pannkakor"]),
    rule(En, K::Search, "search the web for TEXT", &[
        "(search|search for|look up|google|search web for|web search|web search for|search online for|find) {text}",
    ], &["search for weather stockholm", "google rust egui", "look up pancake recipes"]),
    rule(Sv, K::OpenApp, "open an installed program", &[
        "(öppna|starta|kör|öppna upp|dra igång|sätt igång) [appen|programmet|applikationen|program|app|programmet som heter] {text}",
    ], &["öppna spotify", "starta thunderbird", "öppna programmet gimp", "kör steam"]),
    rule(En, K::OpenApp, "open an installed program", &[
        "(open|start|launch|run|fire up|open up) [app|application|program] {text}",
    ], &["open spotify", "launch thunderbird", "start the gimp", "run steam"]),
];

// ── normalisation ─────────────────────────────────────────────────────────

/// Lower case, å/ä/ö and other accents folded, apostrophes removed.
fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        match c {
            'å' | 'ä' | 'à' | 'á' | 'â' | 'æ' => out.push('a'),
            'ö' | 'ø' | 'ó' | 'ò' | 'ô' => out.push('o'),
            'é' | 'è' | 'ê' | 'ë' => out.push('e'),
            'ü' | 'ú' | 'ù' => out.push('u'),
            'í' | 'ì' | 'ï' => out.push('i'),
            '\'' | '’' | '`' | '´' => {}
            c => out.push(c),
        }
    }
    out
}

#[derive(Clone, Debug)]
struct Tok {
    w: String,
    /// Index of the original whitespace word it came from.
    word: usize,
}

/// Words of a transcript: original words (for free text) and folded tokens.
fn tokenize(text: &str) -> (Vec<String>, Vec<Tok>) {
    let words: Vec<String> = text.split_whitespace().map(str::to_string).collect();
    let mut toks = Vec::new();
    for (i, word) in words.iter().enumerate() {
        let folded = fold(word);
        let mut cur = String::new();
        let flush = |cur: &mut String, toks: &mut Vec<Tok>| {
            if !cur.is_empty() {
                toks.push(Tok {
                    w: std::mem::take(cur),
                    word: i,
                });
            }
        };
        for c in folded.chars() {
            if c == '%' {
                flush(&mut cur, &mut toks);
                toks.push(Tok {
                    w: "%".into(),
                    word: i,
                });
            } else if c.is_alphanumeric() {
                // "3:e" and "3e" read alike: a letter right after a digit
                // starts a new token ("50procent" too).
                if cur.chars().last().is_some_and(|l| l.is_ascii_digit()) && !c.is_ascii_digit() {
                    flush(&mut cur, &mut toks);
                }
                cur.push(c);
            } else {
                flush(&mut cur, &mut toks);
            }
        }
        flush(&mut cur, &mut toks);
    }
    (words, toks)
}

const HESITATIONS: &[&str] = &[
    "eh", "ehm", "oh", "ah", "aah", "hm", "hmm", "mm", "mhm", "um", "umm", "uh", "uhm", "er",
    "erm", "eeh", "ahm", "oj",
];

/// Removed wherever they are.
const POLITE_ANYWHERE: &[&str] = &["snalla", "please", "pls", "vanligen", "vaenligen", "nu"];

/// Removed at the start (repeatedly), longest first.
const LEADING: &[&[&str]] = &[
    &["skulle", "du", "kunna"],
    &["skulle", "ni", "kunna"],
    &["jag", "vill", "att", "du"],
    &["jag", "skulle", "vilja"],
    &["var", "snall", "och"],
    &["would", "you", "mind"],
    &["i", "would", "like", "to"],
    &["i", "want", "you", "to"],
    &["go", "ahead", "and"],
    &["could", "you"],
    &["can", "you"],
    &["would", "you"],
    &["will", "you"],
    &["i", "want", "to"],
    &["id", "like", "to"],
    &["kan", "du"],
    &["kan", "ni"],
    &["kunde", "du"],
    &["vill", "du"],
    &["jag", "vill"],
    &["var", "god"],
    &["hej", "dator"],
    &["hey", "computer"],
    &["ok", "computer"],
    &["hej"],
    &["halla"],
    &["okej"],
    &["ok"],
    &["okay"],
    &["och"],
    &["sa"],
    &["nu"],
    &["da"],
    &["nirux"],
    &["datorn"],
    &["dator"],
    &["lets"],
    &["hey"],
    &["hi"],
    &["hello"],
    &["so"],
    &["now"],
    &["and"],
    &["computer"],
    &["just"],
];

/// Removed at the end (repeatedly), longest first.
const TRAILING: &[&[&str]] = &[
    &["tack", "sa", "mycket"],
    &["thank", "you"],
    &["right", "now"],
    &["at", "mig"],
    &["for", "me"],
    &["tack"],
    &["tackar"],
    &["thanks"],
    &["nu"],
    &["da"],
    &["direkt"],
    &["now"],
    &["thx"],
];

/// English words that carry nothing for the matcher. Free text keeps them
/// (it is cut from the original words).
const DETERMINERS: &[&str] = &["the", "a", "an", "my", "this", "that", "some"];

fn strip_seq(toks: &mut Vec<Tok>, seqs: &[&[&str]], leading: bool) {
    loop {
        let mut cut = false;
        for seq in seqs {
            let n = seq.len();
            if toks.len() < n {
                continue;
            }
            let window = if leading {
                &toks[..n]
            } else {
                &toks[toks.len() - n..]
            };
            if window.iter().zip(seq.iter()).all(|(t, s)| t.w == *s) {
                if leading {
                    toks.drain(..n);
                } else {
                    toks.truncate(toks.len() - n);
                }
                cut = true;
                break;
            }
        }
        if !cut {
            return;
        }
    }
}

/// Tokens the templates see. `english` also drops determiners.
fn clean(toks: &[Tok], english: bool) -> Vec<Tok> {
    let mut out: Vec<Tok> = toks
        .iter()
        .filter(|t| {
            !HESITATIONS.contains(&t.w.as_str())
                && !POLITE_ANYWHERE.contains(&t.w.as_str())
                && !(english && DETERMINERS.contains(&t.w.as_str()))
        })
        .cloned()
        .collect();
    strip_seq(&mut out, LEADING, true);
    strip_seq(&mut out, TRAILING, false);
    // "kan du snälla öppna": politeness may sit behind a leading phrase.
    strip_seq(&mut out, LEADING, true);
    if !english {
        // After "kan du" Swedish uses the infinitive: "kan du stänga".
        for t in &mut out {
            if let Some((_, imperative)) = INFINITIVES.iter().find(|(inf, _)| *inf == t.w) {
                t.w = imperative.to_string();
            }
        }
    }
    out
}

/// Swedish infinitives whose imperative differs (folded).
const INFINITIVES: &[(&str, &str)] = &[
    ("stanga", "stang"),
    ("lasa", "las"),
    ("byta", "byt"),
    ("satta", "satt"),
    ("stalla", "stall"),
    ("hoja", "hoj"),
    ("sanka", "sank"),
    ("ga", "ga"),
    ("flytta", "flytta"),
    ("slacka", "slack"),
    ("tanda", "tand"),
    ("kora", "kor"),
    ("fotografera", "fotografera"),
    ("logga", "logga"),
    ("skruva", "skruva"),
];

// ── numbers ───────────────────────────────────────────────────────────────

fn unit(w: &str) -> Option<u32> {
    Some(match w {
        "noll" | "zero" => 0,
        "en" | "ett" | "etta" | "ettan" | "forsta" | "one" | "first" | "won" => 1,
        "tva" | "tvaa" | "tvaan" | "andra" | "two" | "second" | "to" | "too" => 2,
        "tre" | "trea" | "trean" | "tredje" | "three" | "third" | "tree" => 3,
        "fyra" | "fyran" | "fjarde" | "four" | "fourth" | "for" | "forth" => 4,
        "fem" | "femma" | "femman" | "femte" | "five" | "fifth" => 5,
        "sex" | "sexa" | "sexan" | "sjatte" | "six" | "sixth" => 6,
        "sju" | "sjua" | "sjuan" | "sjunde" | "seven" | "seventh" => 7,
        "atta" | "attan" | "attonde" | "eight" | "eighth" | "ate" => 8,
        "nio" | "nia" | "nian" | "nionde" | "nine" | "ninth" => 9,
        "tio" | "tia" | "tian" | "tionde" | "ten" | "tenth" => 10,
        "elva" | "elvan" | "eleven" => 11,
        "tolv" | "tolvan" | "twelve" => 12,
        "tretton" | "thirteen" => 13,
        "fjorton" | "fourteen" => 14,
        "femton" | "fifteen" => 15,
        "sexton" | "sixteen" => 16,
        "sjutton" | "seventeen" => 17,
        "arton" | "aderton" | "eighteen" => 18,
        "nitton" | "nineteen" => 19,
        _ => return None,
    })
}

fn tens(w: &str) -> Option<u32> {
    Some(match w {
        "tjugo" | "twenty" => 20,
        "trettio" | "thirty" => 30,
        "fyrtio" | "forty" | "fourty" => 40,
        "femtio" | "fifty" => 50,
        "sextio" | "sixty" => 60,
        "sjuttio" | "seventy" => 70,
        "attio" | "eighty" => 80,
        "nittio" | "ninety" => 90,
        "hundra" | "etthundra" | "hundred" => 100,
        _ => None?,
    })
}

/// A unit word that may follow a tens word ("tjugofem", "twenty five").
fn small_unit(w: &str) -> Option<u32> {
    match w {
        "en" | "ett" | "one" => Some(1),
        "tva" | "two" => Some(2),
        "tre" | "three" => Some(3),
        "fyra" | "four" => Some(4),
        "fem" | "five" => Some(5),
        "sex" | "six" => Some(6),
        "sju" | "seven" => Some(7),
        "atta" | "eight" => Some(8),
        "nio" | "nine" => Some(9),
        _ => None,
    }
}

const ORDINAL_SUFFIX: &[&str] = &["e", "a", "an", "rd", "st", "nd", "th", "dje"];

/// Numbers that start at `toks[i]`, with how many tokens each takes.
fn numbers(toks: &[Tok], i: usize, fuzzy: bool) -> Vec<(u32, usize, bool)> {
    numbers_exact(toks, i, fuzzy)
        .into_iter()
        .map(|(v, n)| (v, n, false))
        .chain(numbers_near(toks, i, fuzzy))
        .collect()
}

fn numbers_exact(toks: &[Tok], i: usize, _fuzzy: bool) -> Vec<(u32, usize)> {
    let Some(t) = toks.get(i) else {
        return Vec::new();
    };
    let w = t.w.as_str();
    let next = toks.get(i + 1).map(|t| t.w.as_str());
    let mut out = Vec::new();
    if !w.is_empty() && w.len() <= 3 && w.chars().all(|c| c.is_ascii_digit()) {
        let v: u32 = w.parse().unwrap_or(u32::MAX);
        if next.is_some_and(|n| ORDINAL_SUFFIX.contains(&n)) {
            out.push((v, 2));
        }
        out.push((v, 1));
        return out;
    }
    // "one hundred", "ett hundra" (a dropped "a" leaves "hundred").
    if matches!(w, "one" | "ett" | "en") && matches!(next, Some("hundred" | "hundra")) {
        out.push((100, 2));
    }
    if let Some(v) = unit(w) {
        out.push((v, 1));
        return out;
    }
    if let Some(t) = tens(w) {
        if t == 100 {
            out.push((100, 1));
            return out;
        }
        if let Some(u) = next.and_then(small_unit) {
            out.push((t + u, 2));
        }
        out.push((t, 1));
        return out;
    }
    // Swedish compounds: "tjugofem", "fyrtiotvå".
    for prefix in [
        "tjugo", "trettio", "fyrtio", "femtio", "sextio", "sjuttio", "attio", "nittio",
    ] {
        if let Some(rest) = w.strip_prefix(prefix) {
            if let (Some(t), Some(u)) = (tens(prefix), small_unit(rest)) {
                out.push((t + u, 1));
            }
        }
    }
    // English with a hyphen was split already; "twentyfive" is rare but cheap.
    for prefix in [
        "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
    ] {
        if let Some(rest) = w.strip_prefix(prefix) {
            if let (Some(t), Some(u)) = (tens(prefix), small_unit(rest)) {
                out.push((t + u, 1));
            }
        }
    }
    out
}

/// "fira" for "fyra": one vowel off, in number words of four letters or more.
fn numbers_near(toks: &[Tok], i: usize, fuzzy: bool) -> Vec<(u32, usize, bool)> {
    let Some(w) = toks.get(i).map(|t| t.w.as_str()) else {
        return Vec::new();
    };
    if !fuzzy || w.len() < 4 || !w.chars().all(char::is_alphabetic) || unit(w).is_some() {
        return Vec::new();
    }
    const WORDS: &[(&str, u32)] = &[
        ("fyra", 4),
        ("femma", 5),
        ("atta", 8),
        ("trean", 3),
        ("tvaan", 2),
        ("forsta", 1),
        ("andra", 2),
        ("tredje", 3),
        ("three", 3),
        ("seven", 7),
        ("eight", 8),
    ];
    WORDS
        .iter()
        .find(|(word, _)| distance(w, word) <= 0.5)
        .map(|(_, v)| vec![(*v, 1, true)])
        .unwrap_or_default()
}

// ── templates ─────────────────────────────────────────────────────────────

#[derive(Debug)]
enum El {
    /// Alternatives (each a word sequence); `true` when optional.
    Alt(Vec<Vec<String>>, bool),
    Num,
    Text,
}

fn compile(pattern: &str) -> Vec<El> {
    let mut out = Vec::new();
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    let alts = |body: &str| -> Vec<Vec<String>> {
        body.split('|')
            .map(|a| {
                let (_, toks) = tokenize(a);
                toks.into_iter().map(|t| t.w).collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };
    // Longest alternative first, so free text after a group starts as late
    // as it can ("sök på nätet efter X" is a search for X).
    let alts = |body: &str| {
        let mut v = alts(body);
        v.sort_by_key(|a| std::cmp::Reverse(a.len()));
        v
    };
    while i < chars.len() {
        match chars[i] {
            ' ' => i += 1,
            open @ ('(' | '[') => {
                let close = if open == '(' { ')' } else { ']' };
                let end = chars[i..]
                    .iter()
                    .position(|c| *c == close)
                    .map(|p| p + i)
                    .unwrap_or_else(|| panic!("unclosed group in {pattern:?}"));
                let body: String = chars[i + 1..end].iter().collect();
                out.push(El::Alt(alts(&body), open == '['));
                i = end + 1;
            }
            '{' => {
                let end = chars[i..]
                    .iter()
                    .position(|c| *c == '}')
                    .map(|p| p + i)
                    .unwrap_or_else(|| panic!("unclosed slot in {pattern:?}"));
                let name: String = chars[i + 1..end].iter().collect();
                out.push(match name.as_str() {
                    "n" => El::Num,
                    "text" => El::Text,
                    other => panic!("unknown slot {other:?} in {pattern:?}"),
                });
                i = end + 1;
            }
            _ => {
                let end = chars[i..]
                    .iter()
                    .position(|c| *c == ' ')
                    .map(|p| p + i)
                    .unwrap_or(chars.len());
                let word: String = chars[i..end].iter().collect();
                out.push(El::Alt(alts(&word), false));
                i = end;
            }
        }
    }
    out
}

#[derive(Clone, Debug, Default)]
struct Caps {
    n: Option<u32>,
    /// Index into the cleaned tokens where `{text}` starts.
    text: Option<usize>,
}

// ── near misses ───────────────────────────────────────────────────────────

/// Swedish retroflexes sound like the plain consonant ("starta" is heard as
/// "stada"); compare without them.
fn sound(w: &str) -> String {
    w.replace("rt", "t").replace("rd", "d")
}

fn vowel(c: char) -> bool {
    matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'y')
}

/// Edit distance where one vowel for another costs half.
fn distance(a: &str, b: &str) -> f32 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<f32> = (0..=b.len()).map(|j| j as f32).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i as f32; b.len() + 1];
        for j in 1..=b.len() {
            let sub = if a[i - 1] == b[j - 1] {
                0.0
            } else if vowel(a[i - 1]) && vowel(b[j - 1]) {
                0.5
            } else {
                1.0
            };
            cur[j] = (prev[j - 1] + sub).min(prev[j] + 1.0).min(cur[j - 1] + 1.0);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Whether a heard word is close enough to a template word. Short words
/// allow one vowel, longer ones a letter or two. Used only when no rule
/// matched exactly.
fn close(heard: &str, want: &str) -> bool {
    close_within(heard, want, 0.0)
}

/// `close`, with `stricter` taken off the allowed distance.
fn close_within(heard: &str, want: &str, stricter: f32) -> bool {
    if heard == want {
        return true;
    }
    // Digits and symbols must be exact.
    if !want.chars().all(char::is_alphabetic) || !heard.chars().all(char::is_alphabetic) {
        return false;
    }
    let allowed = match want.chars().count() {
        // Short words: one vowel at most ("bit" for "byt", never "till"
        // for "tile").
        0..=4 => 0.5,
        5 => 1.0,
        6..=7 => 1.5,
        _ => 2.0,
    };
    distance(&sound(heard), &sound(want)) <= allowed - stricter
}

/// Match the template from token `i`. `fuzzy` is how many near misses
/// (one word) may still be spent: 0 in the exact pass, 1 in the other.
fn go(els: &[El], toks: &[Tok], i: usize, caps: &mut Caps, fuzzy: u8) -> bool {
    let Some((el, rest)) = els.split_first() else {
        return i == toks.len();
    };
    match el {
        El::Alt(alts, optional) => {
            for alt in alts {
                let n = alt.len();
                if n == 0 || i + n > toks.len() {
                    continue;
                }
                let words = toks[i..i + n].iter().zip(alt);
                if words.clone().all(|(t, w)| t.w == *w) && go(rest, toks, i + n, caps, fuzzy) {
                    return true;
                }
                if fuzzy == 0 {
                    continue;
                }
                let misses = words.clone().filter(|(t, w)| t.w != **w).count();
                if misses == 1
                    && words.clone().all(|(t, w)| close(&t.w, w))
                    && go(rest, toks, i + n, caps, fuzzy - 1)
                {
                    return true;
                }
                // Whisper splits compounds: "arbets-yta", "skärm bild".
                if n == 1
                    && i + 2 <= toks.len()
                    && toks[i].w.chars().all(char::is_alphabetic)
                    && toks[i].w.chars().count() >= 3
                    && toks[i + 1].w.chars().count() >= 3
                    && close_within(&format!("{}{}", toks[i].w, toks[i + 1].w), &alt[0], 0.5)
                    && go(rest, toks, i + 2, caps, fuzzy - 1)
                {
                    return true;
                }
            }
            *optional && go(rest, toks, i, caps, fuzzy)
        }
        El::Num => {
            for (v, len, near) in numbers(toks, i, fuzzy > 0) {
                let saved = caps.clone();
                caps.n = Some(v);
                if go(rest, toks, i + len, caps, fuzzy - u8::from(near)) {
                    return true;
                }
                *caps = saved;
            }
            false
        }
        El::Text => {
            if i < toks.len() && rest.is_empty() {
                caps.text = Some(i);
                true
            } else {
                false
            }
        }
    }
}

struct Compiled {
    rule: &'static Rule,
    patterns: Vec<Vec<El>>,
    free_text: bool,
}

fn compiled() -> &'static [Compiled] {
    static C: OnceLock<Vec<Compiled>> = OnceLock::new();
    C.get_or_init(|| {
        RULES
            .iter()
            .map(|rule| {
                let patterns: Vec<Vec<El>> = rule.patterns.iter().map(|p| compile(p)).collect();
                let free_text = patterns
                    .iter()
                    .any(|p| p.iter().any(|e| matches!(e, El::Text)));
                Compiled {
                    rule,
                    patterns,
                    free_text,
                }
            })
            .collect()
    })
}

// ── free text ─────────────────────────────────────────────────────────────

/// The original words from token `start` to the last kept token, with the
/// punctuation around them trimmed.
fn free_text(words: &[String], toks: &[Tok], start: usize) -> String {
    let (Some(first), Some(last)) = (toks.get(start), toks.last()) else {
        return String::new();
    };
    // Words the matcher skipped right before the text ("the", "a") belong
    // to it: "take a note the meeting" keeps "the meeting".
    let floor = start.checked_sub(1).map(|p| toks[p].word + 1).unwrap_or(0);
    let mut from = first.word;
    while from > floor
        && DETERMINERS
            .contains(&fold(&words[from - 1]).trim_matches(|c: char| !c.is_alphanumeric()))
    {
        from -= 1;
    }
    let mut text = words[from..=last.word].join(" ");
    // A leading token may be part of a word ("3:e" never starts free text,
    // but "kom ihåg: köp" leaves the colon).
    text = text
        .trim_matches(|c: char| c.is_whitespace() || ",.:;!?-–—\"'«»”“".contains(c))
        .to_string();
    text
}

fn capitalise(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Words that name the built-in terminal and browser when spoken as an app.
fn builtin_app(folded: &str) -> Option<Intent> {
    match folded {
        "terminal" | "terminalen" | "en terminal" | "konsol" | "konsolen" | "console" => {
            Some(Intent::Terminal)
        }
        "webblasare" | "webblasaren" | "webblasarn" | "browser" | "browsern" | "web browser"
        | "webbrowser" | "internet" => Some(Intent::Browser),
        _ => None,
    }
}

fn build(kind: Kind, caps: &Caps, words: &[String], toks: &[Tok]) -> Option<Intent> {
    let n = caps.n;
    let text = || caps.text.map(|s| free_text(words, toks, s));
    let tag = |n: Option<u32>| n.filter(|v| (1..=10).contains(v));
    let pct = |n: Option<u32>| n.filter(|v| *v <= 100);
    Some(match kind {
        K::Terminal => Intent::Terminal,
        K::Browser => Intent::Browser,
        K::Workspace => Intent::Workspace(tag(n)?),
        K::MoveToWorkspace => Intent::MoveToWorkspace(tag(n)?),
        K::NextWorkspace => Intent::NextWorkspace,
        K::PrevWorkspace => Intent::PrevWorkspace,
        K::CloseWindow => Intent::CloseWindow,
        K::Fullscreen => Intent::Fullscreen,
        K::Float => Intent::Float,
        K::Layout(l) => Intent::Layout(l),
        K::NextLayout => Intent::NextLayout,
        K::Lock => Intent::Lock,
        K::Launcher => Intent::Launcher,
        K::SearchUi => Intent::SearchUi,
        K::WindowJump => Intent::WindowJump,
        K::QuickNoteUi => Intent::QuickNoteUi,
        K::Overview => Intent::Overview,
        K::NightLight(s) => Intent::NightLight(s),
        K::Dnd(s) => Intent::Dnd(s),
        K::NextMonitor => Intent::NextMonitor,
        K::PrevMonitor => Intent::PrevMonitor,
        K::VolumeUp => Intent::VolumeUp,
        K::VolumeDown => Intent::VolumeDown,
        K::VolumeSet => Intent::VolumeSet(pct(n)?),
        K::Mute => Intent::Mute,
        K::Unmute => Intent::Unmute,
        K::MicMute => Intent::MicMute,
        K::MicUnmute => Intent::MicUnmute,
        K::BrightnessUp => Intent::BrightnessUp,
        K::BrightnessDown => Intent::BrightnessDown,
        K::BrightnessSet => Intent::BrightnessSet(pct(n)?.max(1)),
        K::Play => Intent::Play,
        K::Pause => Intent::Pause,
        K::NextTrack => Intent::NextTrack,
        K::PrevTrack => Intent::PrevTrack,
        K::StopMedia => Intent::StopMedia,
        K::Screenshot(s) => Intent::Screenshot(s),
        K::Suspend => Intent::Suspend,
        K::Reboot => Intent::Reboot,
        K::Shutdown => Intent::Shutdown,
        K::Logout => Intent::Logout,
        K::Note => {
            let t = text()?;
            if t.chars().filter(|c| c.is_alphanumeric()).count() < 2 {
                return None;
            }
            Intent::Note(capitalise(&t))
        }
        K::Search => {
            let t = text()?;
            if t.chars().filter(|c| c.is_alphanumeric()).count() < 2 {
                return None;
            }
            Intent::Search(t)
        }
        K::OpenApp => {
            let start = caps.text?;
            let rest: Vec<&str> = toks[start..].iter().map(|t| t.w.as_str()).collect();
            // "starta om" is a restart, never an app called "om".
            if rest.first().is_some_and(|w| *w == "om" || *w == "igen") {
                return None;
            }
            // "Starta datorn" is a misheard "restart the computer", never
            // a program.
            if matches!(
                rest.as_slice(),
                ["datorn" | "dator" | "computer" | "systemet" | "system"]
            ) {
                return None;
            }
            // An app name is short; a sentence is not a name.
            if rest.len() > 4 || rest.iter().all(|w| w.chars().all(|c| c.is_ascii_digit())) {
                return None;
            }
            if let Some(builtin) = builtin_app(&rest.join(" ")) {
                return Some(builtin);
            }
            let name = free_text(words, toks, start);
            if name.chars().filter(|c| c.is_alphanumeric()).count() < 2 {
                return None;
            }
            Intent::OpenApp(name)
        }
    })
}

/// Match a transcript. `prefer` is tried first (the dictation language).
pub fn parse(text: &str, prefer: Lang) -> Option<Parsed> {
    let (words, toks) = tokenize(text);
    if toks.is_empty() {
        return None;
    }
    let sv_toks = clean(&toks, false);
    let en_toks = clean(&toks, true);
    let order = match prefer {
        Lang::Sv => [Lang::Sv, Lang::En],
        Lang::En => [Lang::En, Lang::Sv],
    };
    // Exact words first (rules without free text, then with it); near
    // misses only when nothing matched exactly, and never for free text.
    for (free, fuzzy) in [(false, false), (true, false), (false, true)] {
        for lang in order {
            let toks = if lang == Lang::En { &en_toks } else { &sv_toks };
            if toks.is_empty() {
                continue;
            }
            for c in compiled()
                .iter()
                .filter(|c| c.free_text == free && c.rule.lang == lang)
            {
                for p in &c.patterns {
                    let mut caps = Caps::default();
                    // A near miss needs exact words around it.
                    if fuzzy && toks.len() < 2 {
                        continue;
                    }
                    if go(p, toks, 0, &mut caps, u8::from(fuzzy)) {
                        if let Some(intent) = build(c.rule.kind, &caps, &words, toks) {
                            return Some(Parsed { intent, lang });
                        }
                    }
                }
            }
        }
    }
    None
}

/// The language a transcript is probably in, for "not understood".
pub fn guess_lang(text: &str, fallback: Lang) -> Lang {
    const SV: &[&str] = &[
        "och", "att", "det", "jag", "du", "inte", "en", "ett", "till", "pa", "av", "med", "oppna",
        "stang", "starta", "kan", "ar", "har", "som", "for", "vad",
    ];
    const EN: &[&str] = &[
        "the", "and", "to", "of", "is", "it", "you", "open", "close", "start", "please", "can",
        "what", "my", "on", "off", "with",
    ];
    if text
        .chars()
        .any(|c| matches!(c, 'å' | 'ä' | 'ö' | 'Å' | 'Ä' | 'Ö'))
    {
        return Lang::Sv;
    }
    let (_, toks) = tokenize(text);
    let sv = toks.iter().filter(|t| SV.contains(&t.w.as_str())).count();
    let en = toks.iter().filter(|t| EN.contains(&t.w.as_str())).count();
    match sv.cmp(&en) {
        std::cmp::Ordering::Greater => Lang::Sv,
        std::cmp::Ordering::Less => Lang::En,
        std::cmp::Ordering::Equal => fallback,
    }
}

// ── what runs ─────────────────────────────────────────────────────────────

/// How a command's process is treated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Run {
    /// Wait for it (bounded) and report a failure.
    Wait,
    /// It opens a window and stays: started is success.
    Ui,
    /// `nirulauncher --launch-best`: read its first line (`launched\tName`).
    LaunchBest,
}

/// One program and its arguments; never a shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub argv: Vec<String>,
    /// Text for standard input (a note).
    pub stdin: Option<String>,
    pub run: Run,
    /// Asks before running (close window, power).
    pub dangerous: bool,
    /// Hide the overlay before it runs (screenshots, lock, power).
    pub hide_overlay: bool,
    /// Run after this delay (a screenshot waits for the overlay to go).
    pub delay_ms: u64,
    /// For an older `niructl` without `do`: the same thing another way.
    pub fallback: Option<Vec<String>>,
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn wm(action: &str) -> Vec<String> {
    let mut v = args(&["niructl", "do"]);
    v.extend(action.split_whitespace().map(str::to_string));
    v
}

impl Intent {
    /// A short name for logs and `--json` (never the free text).
    pub fn name(&self) -> String {
        match self {
            Intent::Workspace(n) => format!("workspace {n}"),
            Intent::MoveToWorkspace(n) => format!("move-to-workspace {n}"),
            Intent::Layout(l) => format!("layout {l}"),
            Intent::NightLight(s) => format!("nightlight {}", s.word()),
            Intent::Dnd(s) => format!("dnd {}", s.word()),
            Intent::VolumeSet(n) => format!("volume {n}"),
            Intent::BrightnessSet(n) => format!("brightness {n}"),
            Intent::Screenshot(s) => format!("screenshot {}", format!("{s:?}").to_lowercase()),
            Intent::OpenApp(_) => "open-app".into(),
            Intent::Note(_) => "note".into(),
            Intent::Search(_) => "search".into(),
            other => {
                let debug = format!("{other:?}");
                let mut out = String::new();
                for (i, c) in debug.chars().enumerate() {
                    if c.is_uppercase() && i > 0 {
                        out.push('-');
                    }
                    out.extend(c.to_lowercase());
                }
                out
            }
        }
    }

    pub fn dangerous(&self) -> bool {
        matches!(
            self,
            Intent::CloseWindow
                | Intent::Suspend
                | Intent::Reboot
                | Intent::Shutdown
                | Intent::Logout
        )
    }

    pub fn plan(&self) -> Plan {
        let mut plan = Plan {
            argv: Vec::new(),
            stdin: None,
            run: Run::Wait,
            dangerous: self.dangerous(),
            hide_overlay: false,
            delay_ms: 0,
            fallback: None,
        };
        plan.argv = match self {
            Intent::Terminal => wm("terminal"),
            Intent::Browser => wm("browser"),
            Intent::Workspace(n) => wm(&format!("tag {n}")),
            Intent::MoveToWorkspace(n) => wm(&format!("movetotag {n}")),
            Intent::NextWorkspace => wm("tag_next"),
            Intent::PrevWorkspace => wm("tag_prev"),
            Intent::CloseWindow => wm("close"),
            Intent::Fullscreen => wm("toggle_fullscreen"),
            Intent::Float => wm("toggle_float"),
            Intent::Layout(l) => {
                plan.fallback = Some(args(&["niructl", "layout", l]));
                wm(&format!("layout {l}"))
            }
            Intent::NextLayout => wm("layout_next"),
            Intent::Lock => {
                plan.hide_overlay = true;
                plan.fallback = Some(args(&["niructl", "lock", "--manual"]));
                wm("lock")
            }
            Intent::Launcher => wm("launcher"),
            Intent::SearchUi => wm("launcher_search"),
            Intent::WindowJump => {
                plan.hide_overlay = true;
                wm("window_hints")
            }
            Intent::QuickNoteUi => wm("quick_note"),
            Intent::Overview => {
                // The overview captures the screen: the overlay goes first.
                plan.hide_overlay = true;
                plan.delay_ms = 150;
                wm("overview")
            }
            Intent::NightLight(s) => args(&["niructl", "nightlight", s.word()]),
            Intent::Dnd(s) => args(&["niructl", "dnd", s.word()]),
            Intent::NextMonitor => wm("focusmon 1"),
            Intent::PrevMonitor => wm("focusmon -1"),
            Intent::OpenApp(q) => {
                plan.run = Run::LaunchBest;
                vec![
                    "nirulauncher".into(),
                    "--apps".into(),
                    "--query".into(),
                    q.clone(),
                    "--launch-best".into(),
                ]
            }
            Intent::VolumeUp => args(&["niructl", "audio", "volume", "up"]),
            Intent::VolumeDown => args(&["niructl", "audio", "volume", "down"]),
            Intent::VolumeSet(n) => vec![
                "wpctl".into(),
                "set-volume".into(),
                "@DEFAULT_AUDIO_SINK@".into(),
                format!("{n}%"),
            ],
            Intent::Mute => args(&["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "1"]),
            Intent::Unmute => args(&["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "0"]),
            Intent::MicMute => args(&["niructl", "audio", "mic-mute", "on"]),
            Intent::MicUnmute => args(&["niructl", "audio", "mic-mute", "off"]),
            Intent::BrightnessUp => args(&["niructl", "brightness", "up"]),
            Intent::BrightnessDown => args(&["niructl", "brightness", "down"]),
            Intent::BrightnessSet(n) => vec![
                "niructl".into(),
                "brightness".into(),
                "set".into(),
                n.to_string(),
            ],
            Intent::Play => args(&["playerctl", "play"]),
            Intent::Pause => args(&["playerctl", "pause"]),
            Intent::NextTrack => args(&["playerctl", "next"]),
            Intent::PrevTrack => args(&["playerctl", "previous"]),
            Intent::StopMedia => args(&["playerctl", "stop"]),
            Intent::Screenshot(s) => {
                plan.hide_overlay = true;
                plan.delay_ms = 350;
                plan.run = Run::Ui;
                let flag = match s {
                    Shot::Full => "--full",
                    Shot::Region => "--region",
                    Shot::Window => "--window",
                };
                args(&["nirushot", flag])
            }
            Intent::Note(t) => {
                plan.stdin = Some(t.clone());
                args(&["nirulauncher", "--note", "--stdin"])
            }
            Intent::Search(t) => {
                plan.run = Run::Ui;
                vec!["nirulauncher".into(), "--search".into(), t.clone()]
            }
            Intent::Suspend => {
                plan.hide_overlay = true;
                args(&["niructl", "suspend"])
            }
            Intent::Reboot => {
                plan.hide_overlay = true;
                args(&["niructl", "reboot"])
            }
            Intent::Shutdown => {
                plan.hide_overlay = true;
                args(&["niructl", "shutdown"])
            }
            Intent::Logout => {
                plan.hide_overlay = true;
                args(&["niructl", "logout"])
            }
        };
        plan
    }

    /// What the overlay says while it runs ("Öppnar terminal").
    pub fn say(&self, lang: Lang) -> String {
        let l = lang;
        match self {
            Intent::Terminal => l.pick("Öppnar terminal", "Opening terminal"),
            Intent::Browser => l.pick("Öppnar webbläsaren", "Opening the browser"),
            Intent::Workspace(n) => match l {
                Lang::Sv => format!("Arbetsyta {n}"),
                Lang::En => format!("Workspace {n}"),
            },
            Intent::MoveToWorkspace(n) => match l {
                Lang::Sv => format!("Flyttar fönstret till arbetsyta {n}"),
                Lang::En => format!("Moving the window to workspace {n}"),
            },
            Intent::NextWorkspace => l.pick("Nästa arbetsyta", "Next workspace"),
            Intent::PrevWorkspace => l.pick("Föregående arbetsyta", "Previous workspace"),
            Intent::CloseWindow => l.pick("Stänger fönstret", "Closing the window"),
            Intent::Fullscreen => l.pick("Helskärm", "Fullscreen"),
            Intent::Float => l.pick("Flytande fönster", "Floating window"),
            Intent::Layout(name) => {
                let sv = match *name {
                    "tile" => "kakel",
                    "monocle" => "monokel",
                    "floating" => "flytande",
                    "centered" => "centrerad",
                    "fibonacci" => "spiral",
                    "scroll" => "rullande",
                    other => other,
                };
                match l {
                    Lang::Sv => format!("Layout: {sv}"),
                    Lang::En => format!("Layout: {name}"),
                }
            }
            Intent::NextLayout => l.pick("Nästa layout", "Next layout"),
            Intent::Lock => l.pick("Låser skärmen", "Locking the screen"),
            Intent::Launcher => l.pick("Öppnar programmenyn", "Opening the launcher"),
            Intent::SearchUi => l.pick("Öppnar sök", "Opening search"),
            Intent::WindowJump => l.pick("Fönsterhopp", "Window jump"),
            Intent::QuickNoteUi => l.pick("Ny anteckning", "New note"),
            Intent::Overview => l.pick("Översikt", "Overview"),
            Intent::NightLight(Switch::On) => l.pick("Nattljus på", "Night light on"),
            Intent::NightLight(Switch::Off) => l.pick("Nattljus av", "Night light off"),
            Intent::NightLight(Switch::Toggle) => l.pick("Växlar nattljus", "Toggling night light"),
            Intent::Dnd(Switch::On) => l.pick("Stör ej på", "Do not disturb on"),
            Intent::Dnd(Switch::Off) => l.pick("Stör ej av", "Do not disturb off"),
            Intent::Dnd(Switch::Toggle) => l.pick("Växlar stör ej", "Toggling do not disturb"),
            Intent::NextMonitor => l.pick("Nästa skärm", "Next screen"),
            Intent::PrevMonitor => l.pick("Föregående skärm", "Previous screen"),
            Intent::OpenApp(q) => match l {
                Lang::Sv => format!("Öppnar {q}"),
                Lang::En => format!("Opening {q}"),
            },
            Intent::VolumeUp => l.pick("Volym upp", "Volume up"),
            Intent::VolumeDown => l.pick("Volym ner", "Volume down"),
            Intent::VolumeSet(n) => match l {
                Lang::Sv => format!("Volym {n} %"),
                Lang::En => format!("Volume {n} %"),
            },
            Intent::Mute => l.pick("Ljud av", "Sound off"),
            Intent::Unmute => l.pick("Ljud på", "Sound on"),
            Intent::MicMute => l.pick("Mikrofon av", "Microphone off"),
            Intent::MicUnmute => l.pick("Mikrofon på", "Microphone on"),
            Intent::BrightnessUp => l.pick("Ljusare", "Brighter"),
            Intent::BrightnessDown => l.pick("Mörkare", "Dimmer"),
            Intent::BrightnessSet(n) => match l {
                Lang::Sv => format!("Ljusstyrka {n} %"),
                Lang::En => format!("Brightness {n} %"),
            },
            Intent::Play => l.pick("Spelar", "Playing"),
            Intent::Pause => l.pick("Pausar", "Pausing"),
            Intent::NextTrack => l.pick("Nästa låt", "Next track"),
            Intent::PrevTrack => l.pick("Föregående låt", "Previous track"),
            Intent::StopMedia => l.pick("Stoppar uppspelningen", "Stopping playback"),
            Intent::Screenshot(_) => l.pick("Skärmbild", "Screenshot"),
            Intent::Note(_) => l.pick("Sparar anteckningen", "Saving the note"),
            Intent::Search(t) => match l {
                Lang::Sv => format!("Söker: {t}"),
                Lang::En => format!("Searching: {t}"),
            },
            Intent::Suspend => l.pick("Viloläge", "Suspending"),
            Intent::Reboot => l.pick("Startar om", "Restarting"),
            Intent::Shutdown => l.pick("Stänger av", "Shutting down"),
            Intent::Logout => l.pick("Loggar ut", "Logging out"),
        }
    }

    /// The question in the confirmation box of a dangerous command.
    pub fn question(&self, lang: Lang) -> String {
        match self {
            Intent::CloseWindow => lang.pick("Stänga fönstret?", "Close the window?"),
            Intent::Suspend => lang.pick("Försätta datorn i viloläge?", "Suspend the computer?"),
            Intent::Reboot => lang.pick("Starta om datorn?", "Restart the computer?"),
            Intent::Shutdown => lang.pick("Stänga av datorn?", "Shut down the computer?"),
            Intent::Logout => lang.pick("Logga ut?", "Log out?"),
            other => other.say(lang),
        }
    }
}

/// Fixed overlay texts.
pub fn text(lang: Lang, what: &str) -> String {
    match (lang, what) {
        (Lang::Sv, "not-understood") => "Förstod inte".into(),
        (Lang::En, "not-understood") => "Not understood".into(),
        (Lang::Sv, "confirm-hint") => "Enter kör · Esc avbryter".into(),
        (Lang::En, "confirm-hint") => "Enter runs · Esc cancels".into(),
        (Lang::Sv, "cancelled") => "Avbrutet".into(),
        (Lang::En, "cancelled") => "Cancelled".into(),
        (Lang::Sv, "title") => "Röstkommando".into(),
        (Lang::En, "title") => "Voice command".into(),
        (Lang::Sv, "note-saved") => "Anteckningen sparad".into(),
        (Lang::En, "note-saved") => "Note saved".into(),
        _ => what.into(),
    }
}

/// `not understood` with the heard text, shortened for the overlay.
pub fn not_understood(lang: Lang, heard: &str) -> String {
    let heard = heard.trim();
    if heard.is_empty() {
        return text(lang, "not-understood");
    }
    let short: String = if heard.chars().count() > 60 {
        heard.chars().take(59).collect::<String>() + "…"
    } else {
        heard.to_string()
    };
    format!("{}: {short}", text(lang, "not-understood"))
}

/// The phrase list for `nirudictate command list` and the manual: the
/// label and examples of each rule, in table order.
pub fn phrase_list() -> Vec<(Lang, &'static str, &'static [&'static str])> {
    RULES
        .iter()
        .map(|r| (r.lang, r.label, r.examples))
        .collect()
}

/// Short vocabulary for whisper's initial prompt, per language.
pub fn prompt_words(lang: Lang) -> &'static str {
    match lang {
        Lang::Sv => "Röstkommandon: öppna terminalen, öppna webbläsaren, gå till arbetsyta tre, flytta fönstret till arbetsyta två, nästa arbetsyta, lås skärmen, ta en skärmbild, höj volymen, sänk volymen, nattljus på, stör ej, helskärm, översikt, anteckning, sök efter, starta om, stäng fönstret.",
        Lang::En => "Voice commands: open terminal, open browser, go to workspace three, move window to workspace two, next workspace, lock screen, take a screenshot, volume up, volume down, night light on, do not disturb, fullscreen, overview, note, search for, restart, close window.",
    }
}

/// Whisper's initial prompt for command mode: the command vocabulary and
/// the programs the user starts most.
pub fn prompt(lang: Lang, apps: &[String]) -> String {
    prompt_styled(PromptStyle::One(lang), apps)
}

/// Which command words whisper's prompt carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptStyle {
    /// The model's language only.
    One(Lang),
    /// Swedish and English phrases side by side (a multilingual model that
    /// should keep English speech English).
    Both,
}

pub fn prompt_styled(style: PromptStyle, apps: &[String]) -> String {
    let lang = match style {
        PromptStyle::One(l) => l,
        PromptStyle::Both => Lang::Sv,
    };
    let mut p = match style {
        PromptStyle::One(l) => prompt_words(l).to_string(),
        PromptStyle::Both => "Öppna terminalen. Open terminal. Gå till arbetsyta tre. Go to workspace three. Flytta fönstret. Move window. Lås skärmen. Lock screen. Höj volymen. Volume up. Ta en skärmbild. Take a screenshot. Nattljus på. Night light on. Stör ej. Do not disturb.".to_string(),
    };
    if !apps.is_empty() {
        p.push_str(match lang {
            Lang::Sv => " Program: ",
            Lang::En => " Programs: ",
        });
        p.push_str(&apps.join(", "));
        p.push('.');
    }
    p
}

#[cfg(test)]
mod tests;
