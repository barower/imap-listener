use simple_logger::SimpleLogger;
use structopt::StructOpt;
use email::rfc2047::decode_rfc2047;
use email::FromHeader;
use std::io::{Read, Write, BufReader};
use std::thread;
use std::process::Command;
use std::fs::File;
use std::error::Error;
use log::{info, trace, warn};

#[derive(StructOpt, Debug)]
#[structopt(name = "idle")]
struct Opt {
    // The server name to connect to
    #[structopt(short, long)]
    server: String,

    // The port to use
    #[structopt(short, long, default_value = "993")]
    port: u16,

    // The account username
    #[structopt(short, long)]
    username: String,

    // The account password. In a production system passwords
    // would normally be in a config or fetched at runtime from
    // a password manager or user prompt and not passed on the
    // command line.
    #[structopt(short = "w", long)]
    password: String,

    // The mailbox to IDLE on
    #[structopt(short, long, default_value = "INBOX")]
    mailbox: String,

    // Refresh rate in seconds
    #[structopt(short, long, default_value = "10")]
    refresh_rate: u64,

    // Json list of allowed people
    #[structopt(long, default_value = "allowed_people.json")]
    allowed_people: String,

    // Json list of subjects that trigger audio
    #[structopt(long, default_value = "triggering_subjects.json")]
    triggering_subjects: String,

    // When is mail considered too old?
    #[structopt(short = "e", long, default_value = "180")]
    mail_expiration_secs: u32,

    // Audio file to play
    #[structopt(short, long, default_value = "~/Music/kanapkiv2.wav")]
    audio_file: String,
}

fn get_subject(header: &imap::types::Fetch<'_>) -> String {
    let envelope = header.envelope().unwrap();

    let _subject = envelope.subject.as_ref().unwrap();
    let subject = String::from_utf8_lossy(_subject);
    if let Some(decoded_subject) = decode_rfc2047(&subject) {
        decoded_subject
    } else {
        subject.to_string()
    }
}

fn get_date(header: &imap::types::Fetch<'_>) -> chrono::DateTime<chrono::Utc> {
    let envelope = header.envelope().unwrap();

    let _date = envelope.date.as_ref().unwrap();
    let date = String::from_utf8_lossy(_date);
    let date = if let Some(decoded_date) = decode_rfc2047(&date) {
        decoded_date
    } else {
        date.to_string()
    };
    FromHeader::from_header(date).unwrap()
}

fn get_from(header: &imap::types::Fetch<'_>) -> String {
    let envelope = header.envelope().unwrap();

    let _from = envelope.from.as_ref().unwrap()[0].name.as_ref().unwrap();
    let from = String::from_utf8_lossy(_from);
    if let Some(decoded_from) = decode_rfc2047(&from) {
        decoded_from
    } else {
        from.to_string()
    }
}

fn person_allowed(person: &String, allowed_people: &Vec<String>) -> bool {
    allowed_people.contains(person)
}

fn parse_json_to_vector(filepath: &String) -> Result<Vec<String>, Box<dyn Error>> {
    let file = File::open(filepath)?;
    let reader = BufReader::new(file);

    let vector = serde_json::from_reader(reader)?;

    Ok(vector)
}

fn subject_is_triggering(subject: &str, allowed_subjects: &Vec<String>) -> bool {
    let subject = subject.to_lowercase();
    allowed_subjects.iter().any(|allowed_subject| edit_distance::edit_distance(&subject, allowed_subject) <= 2)
}

fn mail_too_old(date: chrono::DateTime<chrono::Utc>, limit_secs: u32) -> bool {
    let now_date = chrono::offset::Utc::now();
    let difference_in_seconds = (now_date - date).num_seconds() as u32;
    difference_in_seconds > limit_secs
}

fn move_email<T: Read + Write>(imap: &mut imap::Session<T>, mail_uid: u32, target_folder: &str) {
    imap.copy(mail_uid.to_string(), target_folder).unwrap();
    imap.store(mail_uid.to_string(), "+FLAGS (\\Deleted)").unwrap();
    imap.expunge().unwrap();
}

fn play_notification_sound() {
    thread::spawn(|| {
        let opt = Opt::from_args();
        if let Ok(mut child) = Command::new("play").arg(opt.audio_file).spawn() {
            child.wait().expect("Command wasn't running");
        } else {
            warn!("Failed to run command");
        }
    });
}

fn main() {
    SimpleLogger::new().init().unwrap();

    'connect: loop {
        let opt = Opt::from_args();

        info!("Trying to log in to mailbox");

        let client = match imap::ClientBuilder::new(opt.server.clone(), opt.port).native_tls() {
            Ok(client) => client,
            Err(e) => {
                let dur: std::time::Duration = std::time::Duration::from_secs(opt.refresh_rate);
                info!("Failed to create ClientBuilder: {e:?}");
                info!("Waiting {}s to reconnect", &opt.refresh_rate);
                std::thread::sleep(dur);
                continue 'connect;
            },
        };

        let mut imap: imap::Session<_> = match client.login(opt.username, opt.password) {
            Ok(imap) => imap,
            Err(e) => {
                let dur: std::time::Duration = std::time::Duration::from_secs(opt.refresh_rate);
                info!("Failed to login: {e:?}");
                info!("Waiting {}s to reconnect", &opt.refresh_rate);
                std::thread::sleep(dur);
                continue 'connect;
            },
        };

        // Turn on debug output so we can see the actual traffic coming
        // from the server and how it is handled in our callback.
        // This wouldn't be turned on in a production build, but is helpful
        // in examples and for debugging.
        imap.debug = false;

        imap.select(opt.mailbox).expect("Could not select mailbox");

        'fetch_mails: loop {

            let search_results = match imap.search("UNSEEN") {
                Ok(search_results) => {
                    trace!("Search results: {:?}", &search_results);
                    search_results
                },
                Err(e) => {
                    info!("Failed to fetch emails: {e:?}");
                    continue 'connect;
                }
            };

            for mail_uid in search_results.iter() {
                trace!("Parsing email of UID {mail_uid}");
                let messages = imap.fetch(mail_uid.to_string(), "ENVELOPE").unwrap();
                if let Some(header) = messages.iter().next() {
                    let date = get_date(header);
                    let from = get_from(header);
                    let subject = get_subject(header);
                    let allowed_people = parse_json_to_vector(&opt.allowed_people)
                        .expect(format!("Failed to get {}", opt.allowed_people).as_ref());
                    let triggering_subjects = parse_json_to_vector(&opt.triggering_subjects)
                        .expect(format!("Failed to get {}", opt.triggering_subjects).as_ref());
                    if person_allowed(&from, &allowed_people) && subject_is_triggering(&subject, &triggering_subjects) {
                        move_email(&mut imap, *mail_uid, "Jedzenie");
                        if !mail_too_old(date, opt.mail_expiration_secs) {
                            trace!("New mail from {from}: \"{subject}\"");
                            play_notification_sound();
                        }
                        continue 'fetch_mails; // search for mails again, because mail uid's are at this point invalid
                    }
                } else {
                    warn!("Header not found :(");
                }
            }

            trace!("Waiting for something to arrive");

            let dur: std::time::Duration = std::time::Duration::from_secs(opt.refresh_rate);

            let idle_result = imap.idle().timeout(dur).wait_while(|_response| {
                false
            });

            match idle_result {
                Ok(reason) => trace!("IDLE finished normally {reason:?}"),
                Err(e) => {
                    info!("IDLE finished with error: {e:?}");
                    continue 'connect;
                }
            }

        }
        //imap.logout().expect("Could not log out");
    }
}
