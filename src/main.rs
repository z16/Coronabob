use std::env;
use std::fs;
use std::thread;
use std::time;
use chrono::{DateTime, Utc};
use chrono::offset::TimeZone;
use chrono::serde::ts_seconds;
use async_trait::async_trait;

const LOOP_INTERVAL: u64 = 3600;

#[derive(serde::Deserialize)]
struct TelegramChat {
    id: i64,
    #[serde(default)]
    title: Option<String>,
    #[serde(rename = "type")]
    chat_type: String,
}

#[derive(serde::Deserialize)]
struct TelegramMember {
    id: i64,
}

#[derive(serde::Deserialize)]
struct TelegramMessage {
    //message_id: String,
    chat: TelegramChat,
    #[serde(with = "ts_seconds")]
    date: DateTime<Utc>,
    //#[serde(default)]
    //new_chat_member: Option<TelegramMember>,
    #[serde(default)]
    left_chat_member: Option<TelegramMember>,
    //#[serde(default)]
    //group_chat_created: Option<bool>,
}

#[derive(serde::Deserialize)]
struct TelegramUpdate {
    //update_id: String,
    message: TelegramMessage,
}

#[derive(serde::Deserialize)]
struct TelegramUpdates {
    //ok: bool,
    result: Vec<TelegramUpdate>,
}

#[derive(serde::Deserialize)]
struct CovAttribute {
    #[serde(rename = "Country_Region")]
    region: String,
    #[serde(rename = "Confirmed")]
    confirmed: i64,
    #[serde(rename = "Deaths")]
    deaths: i64,
    #[serde(rename = "Recovered")]
    recovered: i64,
}

#[derive(serde::Deserialize)]
struct CovFeature {
    attributes: CovAttribute,
}

#[derive(serde::Deserialize)]
struct CovData {
    features: Vec<CovFeature>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct BobCovRegion {
    region: String,
    confirmed: i64,
    deaths: i64,
    recovered: i64,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct BobCov {
    #[serde(with = "ts_seconds")]
    date: DateTime<Utc>,
    regions: Vec<BobCovRegion>,
}

type BobData = Vec<BobCov>;

#[derive(serde::Serialize, serde::Deserialize)]
struct TelegramChannel {
    id: i64,
    name: String,
    include: std::collections::HashSet<String>,
    interval: std::time::Duration,
    #[serde(default)]
    test: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct TelegramData {
    #[serde(with = "ts_seconds")]
    date: DateTime<Utc>,
    channels: Vec<TelegramChannel>,
}

struct BobDisplayRegion {
    region: String,
    confirmed: i64,
    deaths: i64,
    recovered: i64,
}

fn format(regions: &Vec<BobDisplayRegion>) -> String {
    let mut lines = vec!();
    lines.push(format!("{:<4}{:>7}{:>6}{:>6}{:>6}{:>6}", "", "C", "D", "R", "D%", "R%"));
    for region in regions.iter() {
        lines.push(format!("{:4}{:7}{:6}{:6}{:5.1}%{:5.1}%",
            region.region,
            region.confirmed,
            region.deaths,
            region.recovered,
            100.0 * region.deaths as f64 / region.confirmed as f64,
            100.0 * region.recovered as f64 / region.confirmed as f64
        ));
    }
    format!("```\n{}\n```", lines.join("\n"))
}

fn load_data<T: serde::de::DeserializeOwned>(path: &str, default: T) -> T {
    if !std::path::Path::new(path).exists() {
        return default;
    }
    let data_string = fs::read_to_string(path).unwrap();
    serde_json::from_str(&data_string).unwrap()
}

fn save_data<T: serde::Serialize>(path: &str, data: &T) {
    fs::write(path, serde_json::to_string_pretty(data).unwrap()).unwrap();
}

struct Client {
    client: reqwest::Client,
}

impl Client {
    async fn get<T>(&self, url: &str) -> T where for<'de> T: serde::Deserialize<'de> {
        println!("Post: {}", url);
        let response = self.client.get(url).send().await.unwrap();
        let json = response.text().await.unwrap();
        serde_json::from_str::<T>(&json).unwrap()
    }

    async fn post_form(&self, url: &str, params: &Vec<(String, String)>) {
        println!("Post: {}", url);
        self.client.post(url).form(params).send().await.unwrap();
    }
}

#[async_trait]
trait Service<'a> {
    async fn process(&self);
}

struct Telegram<'a> {
    base_url: String,
    data: TelegramData,
    cov: &'a BobData,
    client: &'a Client,
    rename: std::collections::HashMap<String, String>,
}

fn url(base: &str, relative: &str) -> String {
    format!("{}{}", base, relative)
}

impl<'a> Telegram<'_> {
    fn url(&self, relative: &str) -> String {
        url(&self.base_url, relative)
    }

    async fn new(key: String, cov: &'a BobData, client: &'a Client, test: bool) -> Telegram<'a> {
        let base_url = format!("https://api.telegram.org/bot{key}/", key = key);
        let bot_id: i64 = key[..key.find(':').unwrap()].to_string().parse().unwrap();

        let mut data: TelegramData = load_data("services/telegram", TelegramData {
            date: Utc.ymd(2020, 01, 01).and_hms(0, 0, 0),
            channels: vec!(),
        });

        let mut channel_ids = std::collections::HashSet::new();
        for channel in data.channels.iter() {
            channel_ids.insert(channel.id.clone());
        }

        let updates: TelegramUpdates = client.get(&url(&base_url, "getUpdates")).await;
        for update in updates.result.iter() {
            let message = &update.message;
            if message.date <= data.date {
                continue;
            }

            let chat = &message.chat;
            if chat.chat_type != "group" {
                continue;
            }

            let left = &message.left_chat_member;
            if left.is_some() && left.as_ref().unwrap().id == bot_id && channel_ids.contains(&chat.id) {
                data.channels.remove(data.channels.iter().position(|channel| channel.id == chat.id).unwrap());
                channel_ids.remove(&chat.id);
            } else if !channel_ids.contains(&chat.id) {
                let mut include = std::collections::HashSet::new();
                include.insert("China".to_string());
                include.insert("Germany".to_string());
                include.insert("US".to_string());
                include.insert("Italy".to_string());
                data.channels.push(TelegramChannel {
                    id: chat.id.clone(),
                    name: chat.title.as_ref().unwrap().clone(),
                    include: include,
                    interval: time::Duration::from_secs(21600),
                    test: false,
                });
                channel_ids.insert(chat.id.clone());
            }
        }

        if !test {
            save_data("services/telegram", &data);
        } else {
            data.channels.retain(|channel| channel.test);
        }

        let mut rename = std::collections::HashMap::new();
        rename.insert("China".to_string(), "CHN".to_string());
        rename.insert("Hong Kong".to_string(), "HKG".to_string());
        rename.insert("United Arab Emirates".to_string(), "ARE".to_string());
        rename.insert("Sri Lanka".to_string(), "LKA".to_string());
        rename.insert("Korea, South".to_string(), "KOR".to_string());
        rename.insert("Australia".to_string(), "AUS".to_string());
        rename.insert("Singapore".to_string(), "SGP".to_string());
        rename.insert("Philippines".to_string(), "PHL".to_string());
        rename.insert("Italy".to_string(), "ITA".to_string());
        rename.insert("Germany".to_string(), "DEU".to_string());
        rename.insert("Japan".to_string(), "JPN".to_string());
        rename.insert("US".to_string(), "USA".to_string());
        rename.insert("Spain".to_string(), "ESP".to_string());
        rename.insert("France".to_string(), "FRA".to_string());

        Telegram {
            base_url: base_url,
            data: data,
            cov: cov,
            client: client,
            rename: rename,
        }
    }

    async fn process(&self, channel: &TelegramChannel) {
        let url = self.url("sendMessage");

        let mut values: Vec<BobDisplayRegion> = vec!();
        let mut total = BobDisplayRegion {
            region: "*".to_string(),
            confirmed: 0,
            deaths: 0,
            recovered: 0,
        };

        for region in self.cov.last().unwrap().regions.iter() {
            total.confirmed += region.confirmed;
            total.deaths += region.deaths;
            total.recovered += region.recovered;

            if channel.include.contains(&region.region) {
                values.push(BobDisplayRegion {
                    region: if self.rename.contains_key(&region.region) {
                        self.rename[&region.region].to_string()
                    } else {
                        region.region.clone()
                    },
                    confirmed: region.confirmed,
                    deaths: region.deaths,
                    recovered: region.recovered,
                });
            }
        }

        values.push(total);
        values.sort_unstable_by(|lhs, rhs| rhs.confirmed.cmp(&lhs.confirmed));
        let message = format(&values);

        let params = vec![
            ("chat_id".to_string(), channel.id.to_string()),
            ("text".to_string(), message.to_string()),
            ("parse_mode".to_string(), "Markdown".to_string()),
            ("disable_notification".to_string(), "true".to_string()),
        ];
        self.client.post_form(&url, &params).await;
    }
}

#[async_trait]
impl Service<'_> for Telegram<'_> {
    async fn process(&self) {
        thread::sleep(time::Duration::from_secs(10));

        let seconds = time::SystemTime::now().duration_since(time::UNIX_EPOCH).unwrap().as_secs() as u64;
        let interval_count = seconds / LOOP_INTERVAL;

        for channel in self.data.channels.iter() {
            let channel_interval_diff = channel.interval.as_secs() / LOOP_INTERVAL;
            let check = interval_count / channel_interval_diff * channel_interval_diff;
            if check != interval_count {
                continue;
            }

            println!("Sending to {}", channel.name);
            self.process(channel).await;
        }
    }
}

async fn get_cov_data(client: &Client) -> BobCov {
    let mut bob_cov = BobCov {
        date: Utc::now(),
        regions: vec!(),
    };

    let cov = client.get::<CovData>("https://services1.arcgis.com/0MSEUqKaxRlEPj5g/arcgis/rest/services/ncov_cases/FeatureServer/2/query?f=json&where=1%3D1&returnGeometry=false&spatialRel=esriSpatialRelIntersects&outFields=*&orderByFields=Confirmed%20desc&outSR=102100&resultOffset=0&resultRecordCount=250&cacheHint=false").await;

    for feature in cov.features.iter() {
        let attribute = &feature.attributes;
        bob_cov.regions.push(BobCovRegion {
            region: attribute.region.clone(),
            confirmed: attribute.confirmed,
            deaths: attribute.deaths,
            recovered: attribute.recovered,
        });
    }

    bob_cov
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();

    let test = !args.contains(&"live".to_string());

    let key = fs::read_to_string("key").expect("No key file found.");
    let key = key.trim();

    fs::create_dir("services").ok();

    println!("Starting! ({})", if cfg!(debug_assertions) { "debug" } else { "release" });

    loop {
        if !test {
            let seconds = time::SystemTime::now().duration_since(time::UNIX_EPOCH).unwrap().as_secs() as u64 % LOOP_INTERVAL as u64;
            let dur = time::Duration::from_secs(LOOP_INTERVAL as u64 - seconds);
            println!("Sleeping for {}", dur.as_secs());
            thread::sleep(dur);
        }

        let mut cov = load_data("cov", vec!());

        let client = Client {
            client: reqwest::Client::new(),
        };

        cov.push(get_cov_data(&client).await);

        {
            let service: Box<dyn Service<'_>> = Box::new(Telegram::new(key.to_string(), &cov, &client, test).await);

            service.process().await;
        }

        if !test {
            save_data("cov", &cov);
        } else {
            break
        }
    }
}

