use super::{flags::CommandFlags, register_command, Exec, Flag, ShellCommand};
use crate::{eval::Value, scope::Scope, utils::format_error};
use crossterm::{
    execute,
    terminal::{DisableLineWrap, EnableLineWrap},
};
use std::io::{stdout, BufReader, Read, Write};
use std::net::{IpAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

struct Whois {
    flags: CommandFlags,
}

impl Whois {
    fn new() -> Self {
        let flags = CommandFlags::with_help();
        Self { flags }
    }

    fn get_whois_server(ip: &IpAddr) -> String {
        match ip {
            IpAddr::V4(_) => "whois.arin.net",
            IpAddr::V6(_) => "whois.ripe.net",
        }
        .to_string()
    }

    fn query_whois(server: &str, ip: &str) -> std::io::Result<String> {
        let mut stream = TcpStream::connect((server, 43))?;
        stream.set_read_timeout(Some(Duration::new(10, 0)))?;
        stream.set_write_timeout(Some(Duration::new(10, 0)))?;

        let query = format!("{}\r\n", ip);
        stream.write_all(query.as_bytes())?;

        let mut reader = BufReader::new(stream);
        let mut response = String::new();
        reader.read_to_string(&mut response)?;

        Ok(response)
    }

    fn whois(scope: &Arc<Scope>, args: &[String]) -> Result<Value, String> {
        let ip_str = &args[0];
        match ip_str.parse::<IpAddr>() {
            Ok(ip) => {
                let whois_server = Whois::get_whois_server(&ip);
                let response = Whois::query_whois(&whois_server, ip_str)
                    .map_err(|e| format_error(scope, ip_str, &args, e))?;

                for line in response.lines() {
                    if let Some(index) = line.find(':') {
                        let (key, value) = line.split_at(index);
                        let key = key.trim().to_string() + ":";
                        let value = value[1..].trim();
                        my_println!("{:16} {}", key, value)?;
                    }
                }
                Ok(Value::success())
            }
            Err(_) => Err(format!("Invalid IP address: {}", ip_str)),
        }
    }
}

impl Exec for Whois {
    fn cli_flags(&self) -> Box<dyn Iterator<Item = &Flag> + '_> {
        Box::new(self.flags.iter())
    }

    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let args = flags.parse(scope, args)?;
        if flags.is_present("help") {
            println!("Usage: whois <IP address>");
            println!("Query WHOIS information for the specified IP address.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if args.is_empty() {
            return Err("Missing IP address".to_string());
        }

        _ = execute!(stdout(), DisableLineWrap);
        let result = Self::whois(scope, &args);
        _ = execute!(stdout(), EnableLineWrap);

        result
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "whois".to_string(),
        inner: Arc::new(Whois::new()),
    });
}
