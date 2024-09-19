use {
    enum_iterator::Sequence, serde::{
        Deserialize,
        Serialize,
    }, std::{borrow::Cow, fmt}
};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, clap::ValueEnum, Sequence)]
#[clap(rename_all = "lower")]
pub enum Locale {
    #[default]
    EN,
    FR,
}

impl Locale {

    pub fn message(&self, message: Message) -> Cow<'static, str> {
        match message {
            Message::InstallerReopenUAC => { //used in installer
                match self {
                    Locale::EN => Cow::Borrowed("The installer has been reopened with admin permissions. Please continue there."),
                    Locale::FR => Cow::Borrowed("L'installateur a été ré-ouvert avec les permissions administrateur. Veuillez continuer dans la nouvelle fenêtre."),
                }
            },
            Message::OpenPj64Button => { // used in gui
                match self {
                    Locale::EN => Cow::Borrowed("Open Project64"),
                    Locale::FR => Cow::Borrowed("Ouvrir Project64"),
                }
            },
            // TODO find a way to translate formatted text somehow
            //Message::AMessageWithParameter(my_int, my_string) => {
            //    match self {
            //        Locale::EN => format!("My integer is: {} and my string is: {}",my_int,my_string).as_str(),
            //        Locale::FR => format!("My integer is: {} and my string is: {}",my_int,my_string).as_str(),
            //    }
            //}
        }
    }
}

impl fmt::Display for Locale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // add local formating for display
            Self::EN => write!(f, "English"),
            Self::FR => write!(f, "Français"),
        }
    }
}

pub enum Message {
    InstallerReopenUAC,
    OpenPj64Button,
    //AMessageWithParameter(i32,String),
}

