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
struct TelegramFile {
    file_id: String,
    //file_unique_id: String,
    //file_size: u32,
    //width: u32,
    //height: u32,
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
    #[serde(default)]
    photo: Option<Vec<TelegramFile>>,
}

#[derive(serde::Deserialize)]
struct TelegramUpdate {
    //update_id: String,
    message: TelegramMessage,
}

#[derive(serde::Deserialize)]
struct TelegramResponse<T> {
    //ok: bool,
    result: Option<T>,
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
    #[serde(default)]
    group: bool,
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

    async fn post_multipart<T: serde::de::DeserializeOwned>(&self, url: &str, form: reqwest::multipart::Form) -> T {
        println!("POST (multipart): {}", url);
        serde_json::from_str(&self.client.post(url).multipart(form).send().await.unwrap().text().await.unwrap()).unwrap()
    }

    async fn post_multipart_void(&self, url: &str, form: reqwest::multipart::Form) {
        println!("POST (multipart): {}", url);
        &self.client.post(url).multipart(form).send().await;
    }
}

#[async_trait]
trait Service<'a> {
    async fn process(&self, image: &Vec<u8>);
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

        let updates: TelegramResponse<Vec<TelegramUpdate>> = client.get(&url(&base_url, "getUpdates")).await;
        for update in updates.result.unwrap().iter() {
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

    async fn process_image(&self, image: &Vec<u8>, channel: &TelegramChannel) -> String {
        let url = self.url("sendPhoto");

        let form = reqwest::multipart::Form::new()
            .part("chat_id", reqwest::multipart::Part::text(channel.id.to_string()))
            .part("disable_notification", reqwest::multipart::Part::text("true"))
            .part("photo", reqwest::multipart::Part::bytes(image.clone()).file_name("test.png"));

        self.client.post_multipart::<TelegramResponse<TelegramMessage>>(&url, form).await.result.unwrap().photo.unwrap()[1].file_id.clone()
    }

    async fn process_id(&self, file_id: &String, channel: &TelegramChannel) {
        let url = self.url("sendPhoto");

        let form = reqwest::multipart::Form::new()
            .part("chat_id", reqwest::multipart::Part::text(channel.id.to_string()))
            .part("disable_notification", reqwest::multipart::Part::text("true"))
            .part("photo", reqwest::multipart::Part::text(file_id.clone()));

        self.client.post_multipart_void(&url, form).await;
    }
}

#[async_trait]
impl Service<'_> for Telegram<'_> {
    async fn process(&self, image: &Vec<u8>) {
        if !self.test {
            thread::sleep(time::Duration::from_secs(10));
        }

        let seconds = time::SystemTime::now().duration_since(time::UNIX_EPOCH).unwrap().as_secs() as u64;
        let interval_count = seconds / LOOP_INTERVAL;

        let mut file_id: Option<String> = None;
        for channel in self.data.channels.iter() {
            let channel_interval_diff = channel.interval.as_secs() / LOOP_INTERVAL;
            let check = interval_count / channel_interval_diff * channel_interval_diff;
            if check != interval_count {
                continue;
            }

            println!("Sending to {}", channel.name);
            match &file_id {
                Some(id) => self.process_id(&id, channel).await,
                None => {
                    file_id = Some(self.process_image(image, channel).await);
                },
            };
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
            group: false,
        });
    }

    bob_cov
}

fn make_image(cov: BobCov) -> Vec<u8> {
    let init_y = 50;
    let init_x = 40;
    let rank_x = init_x + 0;
    let region_x = rank_x + 40;
    let confirmed_x = region_x + 140;
    let deaths_x = confirmed_x + 80;
    let deaths_percent_x = deaths_x + 80;
    let recovered_x = deaths_percent_x + 60;
    let recovered_percent_x = recovered_x + 80;
    let dr_ratio_x = recovered_percent_x + 80;
    let end_x = dr_ratio_x + 50;

    let width = end_x + init_x - 4;
    let height = 800;
    let mut img: image::RgbImage = image::ImageBuffer::new(width, height);
    let font_pixel = image::Rgb([0xC0u8, 0xC0u8, 0xC0u8]);
    let group_pixel = image::Rgb([0xFFu8, 0xA0u8, 0x60u8]);
    let scale = rusttype::Scale {
        x: 18.0,
        y: 18.0,
    };

    let font = rusttype::FontCollection::from_bytes(Vec::from(include_bytes!("Inconsolata.ttf") as &[u8]))
        .unwrap()
        .into_font()
        .unwrap();

    for x in (init_x - 2) .. (width - init_x - 2) {
        img.put_pixel(x, 49, font_pixel);
    }

    let mut draw_entry_full = |x, y, text: &str, pixel| imageproc::drawing::draw_text_mut(&mut img, pixel, x, y, scale, &font, text);
    let mut draw_entry_single = |x, y, text: &str| draw_entry_full(x, y, text, font_pixel);

    draw_entry_single(rank_x, 30, " #");
    draw_entry_single(region_x, 30, "Region");
    draw_entry_single(confirmed_x, 30, "   Cases");
    draw_entry_single(deaths_x, 30, "    Dead");
    draw_entry_single(deaths_percent_x, 30, "    %");
    draw_entry_single(recovered_x, 30, "  Healed");
    draw_entry_single(recovered_percent_x, 30, "    %");
    draw_entry_single(dr_ratio_x, 30, "D/H %");

    let mut offset = 0;
    let mut draw_region = |y, index, region: &BobCovRegion| {
        let pixel = if region.group { group_pixel } else { font_pixel };
        if region.group {
            offset += 1;
            draw_entry_full(rank_x, y, "", pixel);
        } else {
            draw_entry_full(rank_x, y, &format!("{:2}", index - offset + 1), pixel);
        }
        draw_entry_full(region_x, y, &region.region, pixel);
        draw_entry_full(confirmed_x, y, &format!("{:8}", region.confirmed), pixel);
        draw_entry_full(deaths_x, y, &format!("{:8}", region.deaths), pixel);
        draw_entry_full(deaths_percent_x, y, &format!("{:4.1}%", 100.0 * region.deaths as f64 / region.confirmed as f64), pixel);
        draw_entry_full(recovered_x, y, &format!("{:8}", region.recovered), pixel);
        draw_entry_full(recovered_percent_x, y, &format!("{:4.1}%", 100.0 * region.recovered as f64 / region.confirmed as f64), pixel);
        draw_entry_full(dr_ratio_x, y, &format!("{:4.1}%", 100.0 * region.deaths as f64 / (region.deaths + region.recovered) as f64), pixel);
    };

    let mut regions = vec!();

    let mut total = BobCovRegion {
        region: "*".to_string(),
        confirmed: 0,
        deaths: 0,
        recovered: 0,
        group: true,
    };

    let mut groups = std::collections::HashMap::new();

    let mut eu = std::collections::HashSet::new();
    eu.insert("Austria".to_string());
    eu.insert("Belgium".to_string());
    eu.insert("Bulgaria".to_string());
    eu.insert("Croatia".to_string());
    eu.insert("Cyprus".to_string());
    eu.insert("Czechia".to_string());
    eu.insert("Denmark".to_string());
    eu.insert("Estonia".to_string());
    eu.insert("Finland".to_string());
    eu.insert("France".to_string());
    eu.insert("Germany".to_string());
    eu.insert("Greece".to_string());
    eu.insert("Hungary".to_string());
    eu.insert("Ireland".to_string());
    eu.insert("Italy".to_string());
    eu.insert("Latvia".to_string());
    eu.insert("Lithuania".to_string());
    eu.insert("Luxembourg".to_string());
    eu.insert("Malta".to_string());
    eu.insert("Netherlands".to_string());
    eu.insert("Poland".to_string());
    eu.insert("Portugal".to_string());
    eu.insert("Romania".to_string());
    eu.insert("Slovakia".to_string());
    eu.insert("Slovenia".to_string());
    eu.insert("Spain".to_string());
    eu.insert("Sweden".to_string());

    groups.insert("EU".to_string(), eu);

    let mut group_regions = std::collections::HashMap::new();

    for label in groups.keys() {
        group_regions.insert(label, BobCovRegion {
            region: label.to_string(),
            confirmed: 0,
            deaths: 0,
            recovered: 0,
            group: true,
        });
    }

    for region in cov.regions.iter() {
        regions.push(region);

        total.confirmed += region.confirmed;
        total.deaths += region.deaths;
        total.recovered += region.recovered;

        for (label, group) in groups.iter() {
            if group.contains(&region.region) {
                match group_regions.get_mut(label) {
                    Some(group_region) => {
                        group_region.confirmed += region.confirmed;
                        group_region.deaths += region.deaths;
                        group_region.recovered += region.recovered;
                    },
                    None => {},
                };
            }
        }
    }

    regions.push(&total);

    for group in group_regions.values() {
        regions.push(group);
    }

    regions.sort_unstable_by(|lhs, rhs| rhs.confirmed.cmp(&lhs.confirmed));

    for (index, region) in regions.iter().enumerate() {
        let y = init_y + (index as u32) * 20;
        if height - y < 50 {
            break;
        }

        draw_region(y, index, region);
    }

    let mut buffer = Vec::new();
    image::DynamicImage::ImageRgb8(img).write_to(&mut buffer, image::ImageOutputFormat::Png).unwrap();

    buffer
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
        let image = make_image(cov);

        let service: Box<dyn Service<'_>> = Box::new(Telegram::new(key.to_string(), &client, test).await);
        service.process(&image).await;

        if test {
            break;
        }
    }
}

