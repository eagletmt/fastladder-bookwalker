extern crate hyper;
extern crate hyper_rustls;
extern crate serde;
extern crate serde_json;
extern crate select;
extern crate url;
#[macro_use]
extern crate clap;

use select::predicate::Predicate;
use std::error::Error;
use std::io::Read;
use std::io::Write;

fn main() {
    let app = clap::App::new("fastladder-bookwalker")
        .version(crate_version!())
        .about("Post bookwalker feeds to fastladder")
        .arg(clap::Arg::with_name("dry-run").long("dry-run").short("n"))
        .subcommand(clap::SubCommand::with_name("new")
                        .about("Get newly released books")
                        .arg(clap::Arg::with_name("ID")
                                 .required(true)
                                 .multiple(true)
                                 .help("ID (st1, st2, ct1, ct2, ...)")))
        .subcommand(clap::SubCommand::with_name("schedule")
                        .about("Get scheduled books")
                        .arg(clap::Arg::with_name("ID")
                                 .required(true)
                                 .multiple(true)
                                 .help("ID (st1, st2, ct1, ct2, ...)")));
    let matches = app.clone().get_matches();
    let dry_run = matches.is_present("dry-run");
    let client = BookwalkerClient::new(url::Url::parse("https://bookwalker.jp").unwrap());

    match run_subcommand(client, &app, matches.subcommand()) {
        Ok(feeds) => {
            if dry_run {
                println!("{}",
                         serde_json::to_string(&feeds).expect("Unable to encode feeds into JSON"));
            } else {
                let api_key = std::env::var("FASTLADDER_API_KEY")
                    .expect("FASTLADDER_API_KEY is required to post feeds");
                let fastladder_url = std::env::var("FASTLADDER_URL")
                    .expect("FASTLADDER_URL is required to post feeds");
                let fastladder = Fastladder::new(url::Url::parse(&fastladder_url)
                                                     .expect("Unparsable FASTLADDER_URL"),
                                                 api_key);
                match fastladder.post_feeds(&feeds) {
                    Ok(_) => (),
                    Err(msg) => {
                        let _ = writeln!(&mut std::io::stderr(), "{}", msg);
                        std::process::exit(1);
                    }
                }
            }
        }
        Err(msg) => {
            let _ = writeln!(&mut std::io::stderr(), "{}", msg);
            std::process::exit(1);
        }
    }
}

#[derive(Debug)]
struct Feed {
    feedlink: String,
    feedtitle: String,
    author: String,
    title: String,
    thumb_url: url::Url,
    link: String,
    shop: String,
    price: Option<String>,
    category: String,
    guid: String,
}

impl serde::Serialize for Feed {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where S: serde::Serializer
    {
        use serde::ser::SerializeStruct;

        let mut struc = try!(serializer.serialize_struct("Feed", 8));
        try!(struc.serialize_field("feedlink", &self.feedlink));
        try!(struc.serialize_field("feedtitle", &self.feedtitle));
        try!(struc.serialize_field("author", &self.author));
        try!(struc.serialize_field("title", &self.title));
        let mut body = format!("<img src=\"{}\"/><p>{}</p><p>{}</p>",
                               self.thumb_url,
                               self.author,
                               self.shop);
        if let Some(ref price) = self.price {
            body.push_str("<p>");
            body.push_str(price);
            body.push_str("</p>");
        }
        try!(struc.serialize_field("body", &body));
        try!(struc.serialize_field("link", &self.link));
        try!(struc.serialize_field("category", &self.category));
        try!(struc.serialize_field("guid", &self.guid));
        struc.end()
    }
}

#[derive(Debug)]
struct BookwalkerClient {
    base_url: url::Url,
    client: hyper::Client,
}

fn run_subcommand(client: BookwalkerClient,
                  app: &clap::App,
                  subcommand: (&str, Option<&clap::ArgMatches>))
                  -> Result<Vec<Feed>, String> {
    let mut feeds = Vec::new();

    match subcommand {
        ("new", Some(new_command)) => {
            for id in new_command.values_of("ID").unwrap() {
                feeds.append(&mut try!(client.get_new_books(id)));
            }
        }
        ("schedule", Some(schedule_command)) => {
            for id in schedule_command.values_of("ID").unwrap() {
                feeds.append(&mut try!(client.get_schedule_books(id)));
            }
        }
        _ => {
            let _ = app.write_help(&mut std::io::stderr());
            let _ = std::io::stderr().write(b"\n");
            std::process::exit(1);
        }
    };

    return Ok(feeds);
}


impl BookwalkerClient {
    fn new(base_url: url::Url) -> BookwalkerClient {
        let tls = hyper_rustls::TlsClient::new();
        let mut client = hyper::Client::with_connector(hyper::net::HttpsConnector::new(tls));
        client.set_redirect_policy(hyper::client::RedirectPolicy::FollowNone);
        return BookwalkerClient {
                   base_url: base_url,
                   client: client,
               };
    }

    fn get_new_books(&self, id: &str) -> Result<Vec<Feed>, String> {
        let path = format!("/new/{}/?list=0", id);
        match self.base_url.join(&path) {
            Ok(url) => self.get_books(url, &format!("BOOK WALKER {}", path)),
            Err(e) => Err(e.description().to_owned()),
        }
    }

    fn get_schedule_books(&self, id: &str) -> Result<Vec<Feed>, String> {
        let path = format!("/schedule/{}/?list=0", id);
        match self.base_url.join(&path) {
            Ok(url) => self.get_books(url, &format!("BOOK WALKER {}", path)),
            Err(e) => Err(e.description().to_owned()),
        }
    }

    fn get_books(&self, url: url::Url, feedtitle: &str) -> Result<Vec<Feed>, String> {
        match self.client.get(url.clone()).send() {
            Ok(mut res) => {
                let mut body = String::new();
                match res.read_to_string(&mut body) {
                    Ok(_) => {
                        if res.status == hyper::status::StatusCode::Ok {
                            return self.extract_books(&url,
                                                      feedtitle,
                                                      select::document::Document::from(&*body));
                        } else {
                            return Err(format!("/bookmark_new_illust.php returned {}: {}",
                                               res.status,
                                               body));
                        }
                    }
                    Err(e) => Err(e.description().to_owned()),
                }
            }
            Err(e) => Err(e.description().to_owned()),
        }
    }

    fn extract_books(&self,
                     url: &url::Url,
                     feedtitle: &str,
                     doc: select::document::Document)
                     -> Result<Vec<Feed>, String> {
        let mut feeds = Vec::new();
        for item in doc.find(select::predicate::Class("bookItemInner")) {
            let h3_node = try!(item.find(select::predicate::Class("img-book"))
                                   .next()
                                   .ok_or("Unable to find .img-book node".to_owned()));
            let link_node = try!(h3_node
                                     .find(select::predicate::Name("a"))
                                     .next()
                                     .ok_or("Unable to find .img-book a node"));
            let link = try!(link_node
                                .attr("href")
                                .ok_or("href does not exist in .img-book a node"));
            let img_node = try!(link_node
                                    .find(select::predicate::Name("img"))
                                    .next()
                                    .ok_or("Unable to find .img-book a img node"));
            let img = try!(img_node
                               .attr("src")
                               .ok_or("src does not exist in .img-book a img node"));
            let thumb_url = url::Url::parse(img).unwrap();
            let author_node = try!(item.find(select::predicate::Class("book-name"))
                                       .next()
                                       .ok_or("Unable to find .book-name node"));
            let title_node = try!(item.find(select::predicate::Class("book-tl"))
                                      .next()
                                      .ok_or("Unable to find .book-tl node"));
            let shop_node = try!(item.find(select::predicate::Class("shop-name"))
                                     .next()
                                     .ok_or("Unable to find .shop-name node"));
            let price = item.find(select::predicate::Class("book-price")
                                      .or(select::predicate::Class("book-series")))
                .next()
                .map(|node| node.text());
            feeds.push(Feed {
                           feedlink: url.to_string(),
                           feedtitle: feedtitle.to_string(),
                           author: author_node.text(),
                           title: title_node.text(),
                           thumb_url: thumb_url,
                           link: link.to_owned(),
                           shop: shop_node.text(),
                           price: price,
                           category: "bookwalker".to_owned(),
                           guid: link.to_owned(),
                       });
        }
        return Ok(feeds);
    }
}

struct Fastladder {
    base_url: url::Url,
    api_key: String,
}

impl Fastladder {
    fn new(base_url: url::Url, api_key: String) -> Fastladder {
        return Fastladder {
                   base_url: base_url,
                   api_key: api_key,
               };
    }

    fn post_feeds(&self, feeds: &Vec<Feed>) -> Result<(), String> {
        let tls = hyper_rustls::TlsClient::new();
        let client = hyper::Client::with_connector(hyper::net::HttpsConnector::new(tls));
        let url = self.base_url.join("/rpc/update_feeds").unwrap();
        match serde_json::to_string(feeds) {
            Ok(feeds_json) => {
                let request_body = url::form_urlencoded::Serializer::new(String::new())
                    .append_pair("api_key", &self.api_key)
                    .append_pair("feeds", &feeds_json)
                    .finish();
                match client.post(url).body(&request_body).send() {
                    Ok(mut res) => {
                        let mut response_body = String::new();
                        match res.read_to_string(&mut response_body) {
                            Ok(_) => {
                                if res.status == hyper::status::StatusCode::Ok {
                                    return Ok(());
                                } else {
                                    return Err(format!("fastladder/rpc/update_feeds returned \
                                                        {}: {}",
                                                       res.status,
                                                       response_body));
                                }
                            }
                            Err(e) => Err(e.description().to_owned()),
                        }
                    }
                    Err(e) => Err(e.description().to_owned()),
                }
            }
            Err(e) => Err(e.description().to_owned()),
        }
    }
}
