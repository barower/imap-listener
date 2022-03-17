use structopt::StructOpt;
use email::rfc2047::decode_rfc2047;

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

    #[structopt(
        short = "x",
        long,
        help = "The number of responses to receive before exiting",
        default_value = "5"
        )]
        max_responses: usize,
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

fn main() {
    let opt = Opt::from_args();

    let client = imap::ClientBuilder::new(opt.server.clone(), opt.port)
        .native_tls()
        .expect("Could not connect to imap server");

    let mut imap = client
        .login(opt.username, opt.password)
        .expect("Could not authenticate");

    // Turn on debug output so we can see the actual traffic coming
    // from the server and how it is handled in our callback.
    // This wouldn't be turned on in a production build, but is helpful
    // in examples and for debugging.
    imap.debug = false;

    imap.select(opt.mailbox).expect("Could not select mailbox");

    loop {
        let idle_result = imap.idle().wait_while(|_response| {
            false
            //if let imap::types::UnsolicitedResponse::Recent(no) = response {
            //    println!("Recorded {no} new mails");
            //    false
            //} else {
            //    true
            //}
        });

        match idle_result {
            Ok(reason) => println!("IDLE finished normally {reason:?}"),
            Err(e) => println!("IDLE finished with error {e:?}"),
        }

        let search_results = imap.search("UNSEEN").unwrap();
        println!("Search results: {:?}", &search_results);
        for result in search_results.iter() {
            println!("Parsing email of UID {result}");
            let messages = imap.fetch(result.to_string(), "ENVELOPE").unwrap();
            if let Some(header) = messages.iter().next() {
                let subject = get_subject(header);
                println!("Subject is \"{subject}\"");

                let from = get_from(header);
                println!("Subject is \"{from}\"");
            } else {
                println!("Header not found :(");
            }
        }

    }

    //imap.logout().expect("Could not log out");
}
