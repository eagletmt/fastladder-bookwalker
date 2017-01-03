extern crate hyper;
extern crate rustc_serialize;
extern crate select;
extern crate url;
#[macro_use]
extern crate clap;

use select::predicate::Predicate;
use std::error::Error;
use std::io::Read;
use std::io::Write;

fn main() {
    let app = clap_app!(fastladder_bookwalker =>
        (version: "0.1.0")
        (about: "Post bookwalker feeds to fastladder")
        (@arg dry_run: -n "dry-run")
        (@subcommand new =>
            (about: "Get newly released books")
            (@arg ID: +required +multiple "ID (st1, st2, ct1, ct2, ...)"))
        (@subcommand schedule =>
            (about: "Get scheduled books")
            (@arg ID: +required +multiple "ID (st1, st2, ct1, ct2, ...)"))
    );
    let matches = app.clone().get_matches();
    let dry_run = matches.is_present("dry_run");
    let client = BookwalkerClient::new(url::Url::parse("https://bookwalker.jp").unwrap());

    match run_subcommand(client, &app, matches.subcommand()) {
        Ok(feeds) => {
            if dry_run {
                println!("{}",
                         rustc_serialize::json::encode(&feeds)
                             .expect("Unable to encode feeds into JSON"));
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

impl rustc_serialize::Encodable for Feed {
    fn encode<S: rustc_serialize::Encoder>(&self, s: &mut S) -> Result<(), S::Error> {
        s.emit_struct("Feed", 8, |s| {
            try!(s.emit_struct_field("feedlink", 0, |s| self.feedlink.encode(s)));
            try!(s.emit_struct_field("feedtitle", 1, |s| self.feedtitle.encode(s)));
            try!(s.emit_struct_field("author", 2, |s| self.author.encode(s)));
            try!(s.emit_struct_field("title", 3, |s| self.title.encode(s)));
            try!(s.emit_struct_field("body", 4, |s| {
                let mut body = format!("<img src=\"{}\"/><p>{}</p><p>{}</p>",
                                       self.thumb_url,
                                       self.author,
                                       self.shop);
                if let Some(ref price) = self.price {
                    body.push_str("<p>");
                    body.push_str(price);
                    body.push_str("</p>");
                }
                return body.encode(s);
            }));
            try!(s.emit_struct_field("link", 5, |s| self.link.encode(s)));
            try!(s.emit_struct_field("category", 6, |s| self.category.encode(s)));
            try!(s.emit_struct_field("guid", 7, |s| self.guid.encode(s)));
            Ok(())
        })
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
        let mut client = hyper::Client::new();
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
        for item in doc.find(select::predicate::Class("bookItemInner")).iter() {
            let h3_node = try!(item.find(select::predicate::Class("img-book"))
                .first()
                .ok_or("Unable to find .img-book node".to_owned()));
            let link_node = try!(h3_node.find(select::predicate::Name("a"))
                .first()
                .ok_or("Unable to find .img-book a node"));
            let link = try!(link_node.attr("href")
                .ok_or("href does not exist in .img-book a node"));
            let img_node = try!(link_node.find(select::predicate::Name("img"))
                .first()
                .ok_or("Unable to find .img-book a img node"));
            let img = try!(img_node.attr("src")
                .ok_or("src does not exist in .img-book a img node"));
            let thumb_url = url::Url::parse(img).unwrap();
            let author_node = try!(item.find(select::predicate::Class("book-name"))
                .first()
                .ok_or("Unable to find .book-name node"));
            let title_node = try!(item.find(select::predicate::Class("book-tl"))
                .first()
                .ok_or("Unable to find .book-tl node"));
            let shop_node = try!(item.find(select::predicate::Class("shop-name"))
                .first()
                .ok_or("Unable to find .shop-name node"));
            let price = item.find(select::predicate::Class("book-price")
                    .or(select::predicate::Class("book-series")))
                .first()
                .map(|node| node.text());
            let mut link_url = url::Url::parse(link).unwrap();
            link_url.path_segments_mut().unwrap().pop_if_empty().pop();
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
                guid: link_url.to_string(),
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
        let client = hyper::Client::new();
        let url = self.base_url.join("/rpc/update_feeds").unwrap();
        match rustc_serialize::json::encode(feeds) {
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
