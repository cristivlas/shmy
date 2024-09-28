use super::{flags::CommandFlags, register_command, Exec, Flag, ShellCommand};
use crate::{eval::Value, scope::Scope, utils::format_error};
use crossterm::{
    execute,
    terminal::{DisableLineWrap, EnableLineWrap},
};
use std::io::{stdout, BufRead, BufReader, Write};
use std::sync::Arc;
use std::time::Duration;
use std::{
    io,
    net::{IpAddr, TcpStream},
};

struct Whois {
    flags: CommandFlags,
}

impl Whois {
    fn new() -> Self {
        let mut flags = CommandFlags::with_help();
        flags.add_value('h', "host", "server", "whois server");
        Self { flags }
    }

    fn get_whois_server(ip: &IpAddr) -> &str {
        match ip {
            IpAddr::V4(_) => "whois.ripe.net",
            IpAddr::V6(_) => "whois.arin.net",
        }
    }

    fn query_whois(server: &str, ip: &str) -> io::Result<io::Lines<BufReader<TcpStream>>> {
        let mut stream = TcpStream::connect((server, 43))?;
        stream.set_read_timeout(Some(Duration::new(10, 0)))?;
        stream.set_write_timeout(Some(Duration::new(10, 0)))?;

        let query = format!("{}\r\n", ip);
        stream.write_all(query.as_bytes())?;

        let reader = BufReader::new(stream);
        Ok(reader.lines())
    }

    fn whois(args: &[String], server: Option<&str>) -> Result<Value, String> {
        let ip_str = &args[0];
        match ip_str.parse::<IpAddr>() {
            Ok(ip) => {
                let whois_server = server.unwrap_or(Whois::get_whois_server(&ip));
                let lines = Whois::query_whois(&whois_server, ip_str).map_err(|e| e.to_string())?;

                for line in lines {
                    my_println!("{}", line.map_err(|e| e.to_string())?)?;
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
        let whois_args = flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: whois <IP address>");
            println!("Query WHOIS information for the specified IP address.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if whois_args.is_empty() {
            return Err("Missing IP address".to_string());
        }

        _ = execute!(stdout(), DisableLineWrap);
        let result = Self::whois(&whois_args, flags.value("host"));
        _ = execute!(stdout(), EnableLineWrap);

        Ok(result.map_err(|e| format_error(scope, &whois_args[0], &args, e))?)
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "whois".to_string(),
        inner: Arc::new(Whois::new()),
    });
}