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

#[derive(serde::Serialize, serde::Deserialize)]
struct TelegramChannel {
    id: i64,
    name: String,
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
        println!("GET: {}", url);
        let response = self.client.get(url).send().await.unwrap();
        let json = response.text().await.unwrap();
        serde_json::from_str::<T>(&json).unwrap()
    }

    async fn post_multipart(&self, url: &str, form: reqwest::multipart::Form) {
        println!("POST (multipart): {}", url);
        self.client.post(url).multipart(form).send().await.unwrap();
    }
}

#[async_trait]
trait Service<'a> {
    async fn process(&self, cov: &BobCov);
}

struct Telegram<'a> {
    base_url: String,
    data: TelegramData,
    client: &'a Client,
    test: bool,
}

fn url(base: &str, relative: &str) -> String {
    format!("{}{}", base, relative)
}

impl<'a> Telegram<'_> {
    fn url(&self, relative: &str) -> String {
        url(&self.base_url, relative)
    }

    async fn new(key: String, client: &'a Client, test: bool) -> Telegram<'a> {
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
                data.channels.push(TelegramChannel {
                    id: chat.id.clone(),
                    name: chat.title.as_ref().unwrap().clone(),
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

        Telegram {
            base_url: base_url,
            data: data,
            client: client,
            test: test,
        }
    }

    async fn process(&self, cov: &BobCov, channel: &TelegramChannel) {
        let url = self.url("sendPhoto");

        let width = 580;
        let height = 800;
        let mut img: image::RgbImage = image::ImageBuffer::new(width, height);
        let font_pixel = image::Rgb([0xC0u8, 0xC0u8, 0xC0u8]);
        let scale = rusttype::Scale {
            x: 18.0,
            y: 18.0,
        };

        let font = rusttype::FontCollection::from_bytes(Vec::from(include_bytes!("Inconsolata.ttf") as &[u8]))
            .unwrap()
            .into_font()
            .unwrap();

        let init_y = 50;
        let region_x = 48;
        let confirmed_x = 186;
        let deaths_x = 266;
        let deaths_percent_x = 346;
        let recovered_x = 406;
        let recovered_percent_x = 486;

        let mut total = BobCovRegion {
            region: "*".to_string(),
            confirmed: 0,
            deaths: 0,
            recovered: 0,
        };

        for x in (region_x - 1) .. (width - region_x - 1) {
            img.put_pixel(x, 49, font_pixel);
        }

        let mut draw_entry = |x, y, text: &str| imageproc::drawing::draw_text_mut(&mut img, font_pixel, x, y, scale, &font, text);
        draw_entry(region_x, 30, "Region");
        draw_entry(confirmed_x, 30, "   Cases");
        draw_entry(deaths_x, 30, "    Dead");
        draw_entry(deaths_percent_x, 30, "    %");
        draw_entry(recovered_x, 30, "  Healed");
        draw_entry(recovered_percent_x, 30, "    %");

        let mut draw_region = |y, region: &BobCovRegion| {
            draw_entry(region_x, y, &region.region);
            draw_entry(confirmed_x, y, &format!("{:8}", region.confirmed));
            draw_entry(deaths_x, y, &format!("{:8}", region.deaths));
            draw_entry(deaths_percent_x, y, &format!("{:4.1}%", 100.0 * region.deaths as f64 / region.confirmed as f64));
            draw_entry(recovered_x, y, &format!("{:8}", region.recovered));
            draw_entry(recovered_percent_x, y, &format!("{:4.1}%", 100.0 * region.recovered as f64 / region.confirmed as f64));
        };

        for (index, region) in cov.regions.iter().enumerate() {
            let y = init_y + (index as u32 + 1) * 20;
            if height - y < 50 {
                break;
            }

            total.confirmed += region.confirmed;
            total.deaths += region.deaths;
            total.recovered += region.recovered;

            draw_region(y, region);
        }
        draw_region(init_y, &total);

        let mut buffer = Vec::new();
        image::DynamicImage::ImageRgb8(img).write_to(&mut buffer, image::ImageOutputFormat::Png).unwrap();
        let form = reqwest::multipart::Form::new()
            .part("chat_id", reqwest::multipart::Part::text(channel.id.to_string()))
            .part("disable_notification", reqwest::multipart::Part::text("true"))
            .part("photo", reqwest::multipart::Part::bytes(buffer).file_name("test.png"));

        self.client.post_multipart(&url, form).await;
    }
}

#[async_trait]
impl Service<'_> for Telegram<'_> {
    async fn process(&self, cov: &BobCov) {
        if !self.test {
            thread::sleep(time::Duration::from_secs(10));
        }

        let seconds = time::SystemTime::now().duration_since(time::UNIX_EPOCH).unwrap().as_secs() as u64;
        let interval_count = seconds / LOOP_INTERVAL;

        for channel in self.data.channels.iter() {
            let channel_interval_diff = channel.interval.as_secs() / LOOP_INTERVAL;
            let check = interval_count / channel_interval_diff * channel_interval_diff;
            if check != interval_count {
                continue;
            }

            println!("Sending to {}", channel.name);
            self.process(cov, channel).await;
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

        let client = Client {
            client: reqwest::Client::new(),
        };

        let cov = get_cov_data(&client).await;

        let service: Box<dyn Service<'_>> = Box::new(Telegram::new(key.to_string(), &client, test).await);
        service.process(&cov).await;

        if test {
            break;
        }
    }
}

