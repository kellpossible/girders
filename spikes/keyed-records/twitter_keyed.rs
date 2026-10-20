//! `twitter.json`: a Twitter search API response, string-heavy, wide
//! records (`User` has 40 fields) and one recursive field
//! (`Status.retweeted_status`).
//!
//! Departures from the json-benchmark model, applied to both sides alike:
//! `PrimStr<u64>`/`PrimStr<u32>` (`id_str` etc.) and `Color` are kept as
//! `String`; `empty::Array` (`symbols`) is `[(); 0]`; the `enum_str!`
//! enums are plain fieldless enums.

use girders::prelude::*;
use serde::{Deserialize, Serialize};

pub type LongId = u64;
pub type ShortId = u32;
pub type Indices = (u8, u8);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Twitter {
    pub statuses: Vec<Status>,
    pub search_metadata: SearchMetadata,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Status {
    pub metadata: Metadata,
    pub created_at: String,
    pub id: LongId,
    pub id_str: String,
    pub text: String,
    pub source: String,
    pub truncated: bool,
    pub in_reply_to_status_id: Option<LongId>,
    pub in_reply_to_status_id_str: Option<String>,
    pub in_reply_to_user_id: Option<ShortId>,
    pub in_reply_to_user_id_str: Option<String>,
    pub in_reply_to_screen_name: Option<String>,
    pub user: User,
    pub geo: (),
    pub coordinates: (),
    pub place: (),
    pub contributors: (),
    pub retweeted_status: Option<Box<Status>>,
    pub retweet_count: u32,
    pub favorite_count: u32,
    pub entities: StatusEntities,
    pub favorited: bool,
    pub retweeted: bool,
    pub possibly_sensitive: Option<bool>,
    pub lang: LanguageCode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub result_type: ResultType,
    pub iso_language_code: LanguageCode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct User {
    pub id: ShortId,
    pub id_str: String,
    pub name: String,
    pub screen_name: String,
    pub location: String,
    pub description: String,
    pub url: Option<String>,
    pub entities: UserEntities,
    pub protected: bool,
    pub followers_count: u32,
    pub friends_count: u32,
    pub listed_count: u32,
    pub created_at: String,
    pub favourites_count: u32,
    pub utc_offset: Option<i32>,
    pub time_zone: Option<String>,
    pub geo_enabled: bool,
    pub verified: bool,
    pub statuses_count: u32,
    pub lang: LanguageCode,
    pub contributors_enabled: bool,
    pub is_translator: bool,
    pub is_translation_enabled: bool,
    pub profile_background_color: String,
    pub profile_background_image_url: String,
    pub profile_background_image_url_https: String,
    pub profile_background_tile: bool,
    pub profile_image_url: String,
    pub profile_image_url_https: String,
    pub profile_banner_url: Option<String>,
    pub profile_link_color: String,
    pub profile_sidebar_border_color: String,
    pub profile_sidebar_fill_color: String,
    pub profile_text_color: String,
    pub profile_use_background_image: bool,
    pub default_profile: bool,
    pub default_profile_image: bool,
    pub following: bool,
    pub follow_request_sent: bool,
    pub notifications: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserEntities {
    pub url: Option<UserUrl>,
    pub description: UserEntitiesDescription,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserUrl {
    pub urls: Vec<Url>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Url {
    pub url: String,
    pub expanded_url: String,
    pub display_url: String,
    pub indices: Indices,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserEntitiesDescription {
    pub urls: Vec<Url>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusEntities {
    pub hashtags: Vec<Hashtag>,
    pub symbols: [(); 0],
    pub urls: Vec<Url>,
    pub user_mentions: Vec<UserMention>,
    pub media: Option<Vec<Media>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hashtag {
    pub text: String,
    pub indices: Indices,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserMention {
    pub screen_name: String,
    pub name: String,
    pub id: ShortId,
    pub id_str: String,
    pub indices: Indices,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Media {
    pub id: LongId,
    pub id_str: String,
    pub indices: Indices,
    pub media_url: String,
    pub media_url_https: String,
    pub url: String,
    pub display_url: String,
    pub expanded_url: String,
    #[serde(rename = "type")]
    pub media_type: String,
    pub sizes: Sizes,
    pub source_status_id: Option<LongId>,
    pub source_status_id_str: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sizes {
    pub medium: Size,
    pub small: Size,
    pub thumb: Size,
    pub large: Size,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Size {
    pub w: u16,
    pub h: u16,
    pub resize: Resize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchMetadata {
    pub completed_in: f32,
    pub max_id: LongId,
    pub max_id_str: String,
    pub next_results: String,
    pub query: String,
    pub refresh_url: String,
    pub count: u8,
    pub since_id: LongId,
    pub since_id_str: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Resize {
    #[serde(rename = "fit")]
    Fit,
    #[serde(rename = "crop")]
    Crop,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LanguageCode {
    #[serde(rename = "zh-cn")]
    Cn,
    #[serde(rename = "en")]
    En,
    #[serde(rename = "es")]
    Es,
    #[serde(rename = "it")]
    It,
    #[serde(rename = "ja")]
    Ja,
    #[serde(rename = "zh")]
    Zh,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ResultType {
    #[serde(rename = "recent")]
    Recent,
}

// -- girders codecs --------------------------------------------------------------

fn resize() -> impl Codec<Output = Resize> + Clone {
    string_enum([("fit", Resize::Fit), ("crop", Resize::Crop)])
}

fn language_code() -> impl Codec<Output = LanguageCode> + Clone {
    string_enum([
        ("zh-cn", LanguageCode::Cn),
        ("en", LanguageCode::En),
        ("es", LanguageCode::Es),
        ("it", LanguageCode::It),
        ("ja", LanguageCode::Ja),
        ("zh", LanguageCode::Zh),
    ])
}

fn result_type() -> impl Codec<Output = ResultType> + Clone {
    string_enum([("recent", ResultType::Recent)])
}

fn indices() -> impl Codec<Output = Indices> + Clone {
    tuple2(u8(), u8())
}

/// `[]` only, like json-benchmark's `empty::Array`.
fn empty_array() -> impl Codec<Output = [(); 0]> + Clone {
    try_bimap(
        list(null()),
        |v: Vec<()>| {
            if v.is_empty() {
                Ok([])
            } else {
                Err("expected an empty array")
            }
        },
        |_: &[(); 0]| Vec::new(),
    )
}

fn metadata() -> impl Codec<Output = Metadata> + Clone {
    let r = keyed_record::<Metadata>();
    let (r, k_result_type) = r.field("result_type", result_type(), |x| &x.result_type);
    let (r, k_iso_language_code) = r.field("iso_language_code", language_code(), |x| {
        &x.iso_language_code
    });
    r.build(move |v| Metadata {
        result_type: v.take(k_result_type),
        iso_language_code: v.take(k_iso_language_code),
    })
}

fn url() -> impl Codec<Output = Url> + Clone {
    let r = keyed_record::<Url>();
    let (r, k_url) = r.field("url", string(), |x| &x.url);
    let (r, k_expanded_url) = r.field("expanded_url", string(), |x| &x.expanded_url);
    let (r, k_display_url) = r.field("display_url", string(), |x| &x.display_url);
    let (r, k_indices) = r.field("indices", indices(), |x| &x.indices);
    r.build(move |v| Url {
        url: v.take(k_url),
        expanded_url: v.take(k_expanded_url),
        display_url: v.take(k_display_url),
        indices: v.take(k_indices),
    })
}

fn user_url() -> impl Codec<Output = UserUrl> + Clone {
    let r = keyed_record::<UserUrl>();
    let (r, k_urls) = r.field("urls", list(url()), |x| &x.urls);
    r.build(move |v| UserUrl {
        urls: v.take(k_urls),
    })
}

fn user_entities_description() -> impl Codec<Output = UserEntitiesDescription> + Clone {
    let r = keyed_record::<UserEntitiesDescription>();
    let (r, k_urls) = r.field("urls", list(url()), |x| &x.urls);
    r.build(move |v| UserEntitiesDescription {
        urls: v.take(k_urls),
    })
}

fn user_entities() -> impl Codec<Output = UserEntities> + Clone {
    let r = keyed_record::<UserEntities>();
    let (r, k_url) = r.optional_field("url", user_url(), |x| &x.url);
    let (r, k_description) = r.field("description", user_entities_description(), |x| {
        &x.description
    });
    r.build(move |v| UserEntities {
        url: v.take(k_url),
        description: v.take(k_description),
    })
}

fn user() -> impl Codec<Output = User> + Clone {
    let r = keyed_record::<User>();
    let (r, k_id) = r.field("id", u32(), |x| &x.id);
    let (r, k_id_str) = r.field("id_str", string(), |x| &x.id_str);
    let (r, k_name) = r.field("name", string(), |x| &x.name);
    let (r, k_screen_name) = r.field("screen_name", string(), |x| &x.screen_name);
    let (r, k_location) = r.field("location", string(), |x| &x.location);
    let (r, k_description) = r.field("description", string(), |x| &x.description);
    let (r, k_url) = r.optional_field("url", string(), |x| &x.url);
    let (r, k_entities) = r.field("entities", user_entities(), |x| &x.entities);
    let (r, k_protected) = r.field("protected", bool(), |x| &x.protected);
    let (r, k_followers_count) = r.field("followers_count", u32(), |x| &x.followers_count);
    let (r, k_friends_count) = r.field("friends_count", u32(), |x| &x.friends_count);
    let (r, k_listed_count) = r.field("listed_count", u32(), |x| &x.listed_count);
    let (r, k_created_at) = r.field("created_at", string(), |x| &x.created_at);
    let (r, k_favourites_count) = r.field("favourites_count", u32(), |x| &x.favourites_count);
    let (r, k_utc_offset) = r.optional_field("utc_offset", i32(), |x| &x.utc_offset);
    let (r, k_time_zone) = r.optional_field("time_zone", string(), |x| &x.time_zone);
    let (r, k_geo_enabled) = r.field("geo_enabled", bool(), |x| &x.geo_enabled);
    let (r, k_verified) = r.field("verified", bool(), |x| &x.verified);
    let (r, k_statuses_count) = r.field("statuses_count", u32(), |x| &x.statuses_count);
    let (r, k_lang) = r.field("lang", language_code(), |x| &x.lang);
    let (r, k_contributors_enabled) =
        r.field("contributors_enabled", bool(), |x| &x.contributors_enabled);
    let (r, k_is_translator) = r.field("is_translator", bool(), |x| &x.is_translator);
    let (r, k_is_translation_enabled) = r.field("is_translation_enabled", bool(), |x| {
        &x.is_translation_enabled
    });
    let (r, k_profile_background_color) = r.field("profile_background_color", string(), |x| {
        &x.profile_background_color
    });
    let (r, k_profile_background_image_url) =
        r.field("profile_background_image_url", string(), |x| {
            &x.profile_background_image_url
        });
    let (r, k_profile_background_image_url_https) =
        r.field("profile_background_image_url_https", string(), |x| {
            &x.profile_background_image_url_https
        });
    let (r, k_profile_background_tile) = r.field("profile_background_tile", bool(), |x| {
        &x.profile_background_tile
    });
    let (r, k_profile_image_url) = r.field("profile_image_url", string(), |x| &x.profile_image_url);
    let (r, k_profile_image_url_https) = r.field("profile_image_url_https", string(), |x| {
        &x.profile_image_url_https
    });
    let (r, k_profile_banner_url) =
        r.optional_field("profile_banner_url", string(), |x| &x.profile_banner_url);
    let (r, k_profile_link_color) =
        r.field("profile_link_color", string(), |x| &x.profile_link_color);
    let (r, k_profile_sidebar_border_color) =
        r.field("profile_sidebar_border_color", string(), |x| {
            &x.profile_sidebar_border_color
        });
    let (r, k_profile_sidebar_fill_color) = r.field("profile_sidebar_fill_color", string(), |x| {
        &x.profile_sidebar_fill_color
    });
    let (r, k_profile_text_color) =
        r.field("profile_text_color", string(), |x| &x.profile_text_color);
    let (r, k_profile_use_background_image) =
        r.field("profile_use_background_image", bool(), |x| {
            &x.profile_use_background_image
        });
    let (r, k_default_profile) = r.field("default_profile", bool(), |x| &x.default_profile);
    let (r, k_default_profile_image) = r.field("default_profile_image", bool(), |x| {
        &x.default_profile_image
    });
    let (r, k_following) = r.field("following", bool(), |x| &x.following);
    let (r, k_follow_request_sent) =
        r.field("follow_request_sent", bool(), |x| &x.follow_request_sent);
    let (r, k_notifications) = r.field("notifications", bool(), |x| &x.notifications);
    r.build(move |v| User {
        id: v.take(k_id),
        id_str: v.take(k_id_str),
        name: v.take(k_name),
        screen_name: v.take(k_screen_name),
        location: v.take(k_location),
        description: v.take(k_description),
        url: v.take(k_url),
        entities: v.take(k_entities),
        protected: v.take(k_protected),
        followers_count: v.take(k_followers_count),
        friends_count: v.take(k_friends_count),
        listed_count: v.take(k_listed_count),
        created_at: v.take(k_created_at),
        favourites_count: v.take(k_favourites_count),
        utc_offset: v.take(k_utc_offset),
        time_zone: v.take(k_time_zone),
        geo_enabled: v.take(k_geo_enabled),
        verified: v.take(k_verified),
        statuses_count: v.take(k_statuses_count),
        lang: v.take(k_lang),
        contributors_enabled: v.take(k_contributors_enabled),
        is_translator: v.take(k_is_translator),
        is_translation_enabled: v.take(k_is_translation_enabled),
        profile_background_color: v.take(k_profile_background_color),
        profile_background_image_url: v.take(k_profile_background_image_url),
        profile_background_image_url_https: v.take(k_profile_background_image_url_https),
        profile_background_tile: v.take(k_profile_background_tile),
        profile_image_url: v.take(k_profile_image_url),
        profile_image_url_https: v.take(k_profile_image_url_https),
        profile_banner_url: v.take(k_profile_banner_url),
        profile_link_color: v.take(k_profile_link_color),
        profile_sidebar_border_color: v.take(k_profile_sidebar_border_color),
        profile_sidebar_fill_color: v.take(k_profile_sidebar_fill_color),
        profile_text_color: v.take(k_profile_text_color),
        profile_use_background_image: v.take(k_profile_use_background_image),
        default_profile: v.take(k_default_profile),
        default_profile_image: v.take(k_default_profile_image),
        following: v.take(k_following),
        follow_request_sent: v.take(k_follow_request_sent),
        notifications: v.take(k_notifications),
    })
}

fn hashtag() -> impl Codec<Output = Hashtag> + Clone {
    let r = keyed_record::<Hashtag>();
    let (r, k_text) = r.field("text", string(), |x| &x.text);
    let (r, k_indices) = r.field("indices", indices(), |x| &x.indices);
    r.build(move |v| Hashtag {
        text: v.take(k_text),
        indices: v.take(k_indices),
    })
}

fn user_mention() -> impl Codec<Output = UserMention> + Clone {
    let r = keyed_record::<UserMention>();
    let (r, k_screen_name) = r.field("screen_name", string(), |x| &x.screen_name);
    let (r, k_name) = r.field("name", string(), |x| &x.name);
    let (r, k_id) = r.field("id", u32(), |x| &x.id);
    let (r, k_id_str) = r.field("id_str", string(), |x| &x.id_str);
    let (r, k_indices) = r.field("indices", indices(), |x| &x.indices);
    r.build(move |v| UserMention {
        screen_name: v.take(k_screen_name),
        name: v.take(k_name),
        id: v.take(k_id),
        id_str: v.take(k_id_str),
        indices: v.take(k_indices),
    })
}

fn size() -> impl Codec<Output = Size> + Clone {
    let r = keyed_record::<Size>();
    let (r, k_w) = r.field("w", u16(), |x| &x.w);
    let (r, k_h) = r.field("h", u16(), |x| &x.h);
    let (r, k_resize) = r.field("resize", resize(), |x| &x.resize);
    r.build(move |v| Size {
        w: v.take(k_w),
        h: v.take(k_h),
        resize: v.take(k_resize),
    })
}

fn sizes() -> impl Codec<Output = Sizes> + Clone {
    let r = keyed_record::<Sizes>();
    let (r, k_medium) = r.field("medium", size(), |x| &x.medium);
    let (r, k_small) = r.field("small", size(), |x| &x.small);
    let (r, k_thumb) = r.field("thumb", size(), |x| &x.thumb);
    let (r, k_large) = r.field("large", size(), |x| &x.large);
    r.build(move |v| Sizes {
        medium: v.take(k_medium),
        small: v.take(k_small),
        thumb: v.take(k_thumb),
        large: v.take(k_large),
    })
}

fn media() -> impl Codec<Output = Media> + Clone {
    let r = keyed_record::<Media>();
    let (r, k_id) = r.field("id", u64(), |x| &x.id);
    let (r, k_id_str) = r.field("id_str", string(), |x| &x.id_str);
    let (r, k_indices) = r.field("indices", indices(), |x| &x.indices);
    let (r, k_media_url) = r.field("media_url", string(), |x| &x.media_url);
    let (r, k_media_url_https) = r.field("media_url_https", string(), |x| &x.media_url_https);
    let (r, k_url) = r.field("url", string(), |x| &x.url);
    let (r, k_display_url) = r.field("display_url", string(), |x| &x.display_url);
    let (r, k_expanded_url) = r.field("expanded_url", string(), |x| &x.expanded_url);
    let (r, k_media_type) = r.field("type", string(), |x| &x.media_type);
    let (r, k_sizes) = r.field("sizes", sizes(), |x| &x.sizes);
    let (r, k_source_status_id) =
        r.optional_field("source_status_id", u64(), |x| &x.source_status_id);
    let (r, k_source_status_id_str) = r.optional_field("source_status_id_str", string(), |x| {
        &x.source_status_id_str
    });
    r.build(move |v| Media {
        id: v.take(k_id),
        id_str: v.take(k_id_str),
        indices: v.take(k_indices),
        media_url: v.take(k_media_url),
        media_url_https: v.take(k_media_url_https),
        url: v.take(k_url),
        display_url: v.take(k_display_url),
        expanded_url: v.take(k_expanded_url),
        media_type: v.take(k_media_type),
        sizes: v.take(k_sizes),
        source_status_id: v.take(k_source_status_id),
        source_status_id_str: v.take(k_source_status_id_str),
    })
}

fn status_entities() -> impl Codec<Output = StatusEntities> + Clone {
    let r = keyed_record::<StatusEntities>();
    let (r, k_hashtags) = r.field("hashtags", list(hashtag()), |x| &x.hashtags);
    let (r, k_symbols) = r.field("symbols", empty_array(), |x| &x.symbols);
    let (r, k_urls) = r.field("urls", list(url()), |x| &x.urls);
    let (r, k_user_mentions) = r.field("user_mentions", list(user_mention()), |x| &x.user_mentions);
    let (r, k_media) = r.optional_field("media", list(media()), |x| &x.media);
    r.build(move |v| StatusEntities {
        hashtags: v.take(k_hashtags),
        symbols: v.take(k_symbols),
        urls: v.take(k_urls),
        user_mentions: v.take(k_user_mentions),
        media: v.take(k_media),
    })
}

/// `Status` is recursive through `retweeted_status`.
#[derive(Debug, Clone, Copy)]
struct StatusDef;

impl Recursive for StatusDef {
    type Output = Status;
    fn codec(&self) -> impl Codec<Output = Status> {
        let r = keyed_record::<Status>();
        let (r, k_metadata) = r.field("metadata", metadata(), |x| &x.metadata);
        let (r, k_created_at) = r.field("created_at", string(), |x| &x.created_at);
        let (r, k_id) = r.field("id", u64(), |x| &x.id);
        let (r, k_id_str) = r.field("id_str", string(), |x| &x.id_str);
        let (r, k_text) = r.field("text", string(), |x| &x.text);
        let (r, k_source) = r.field("source", string(), |x| &x.source);
        let (r, k_truncated) = r.field("truncated", bool(), |x| &x.truncated);
        let (r, k_in_reply_to_status_id) =
            r.optional_field("in_reply_to_status_id", u64(), |x| &x.in_reply_to_status_id);
        let (r, k_in_reply_to_status_id_str) =
            r.optional_field("in_reply_to_status_id_str", string(), |x| {
                &x.in_reply_to_status_id_str
            });
        let (r, k_in_reply_to_user_id) =
            r.optional_field("in_reply_to_user_id", u32(), |x| &x.in_reply_to_user_id);
        let (r, k_in_reply_to_user_id_str) =
            r.optional_field("in_reply_to_user_id_str", string(), |x| {
                &x.in_reply_to_user_id_str
            });
        let (r, k_in_reply_to_screen_name) =
            r.optional_field("in_reply_to_screen_name", string(), |x| {
                &x.in_reply_to_screen_name
            });
        let (r, k_user) = r.field("user", user(), |x| &x.user);
        let (r, k_geo) = r.field("geo", null(), |x| &x.geo);
        let (r, k_coordinates) = r.field("coordinates", null(), |x| &x.coordinates);
        let (r, k_place) = r.field("place", null(), |x| &x.place);
        let (r, k_contributors) = r.field("contributors", null(), |x| &x.contributors);
        let (r, k_retweeted_status) =
            r.optional_field("retweeted_status", boxed(recursive(StatusDef)), |x| {
                &x.retweeted_status
            });
        let (r, k_retweet_count) = r.field("retweet_count", u32(), |x| &x.retweet_count);
        let (r, k_favorite_count) = r.field("favorite_count", u32(), |x| &x.favorite_count);
        let (r, k_entities) = r.field("entities", status_entities(), |x| &x.entities);
        let (r, k_favorited) = r.field("favorited", bool(), |x| &x.favorited);
        let (r, k_retweeted) = r.field("retweeted", bool(), |x| &x.retweeted);
        let (r, k_possibly_sensitive) =
            r.optional_field("possibly_sensitive", bool(), |x| &x.possibly_sensitive);
        let (r, k_lang) = r.field("lang", language_code(), |x| &x.lang);
        r.build(move |v| Status {
            metadata: v.take(k_metadata),
            created_at: v.take(k_created_at),
            id: v.take(k_id),
            id_str: v.take(k_id_str),
            text: v.take(k_text),
            source: v.take(k_source),
            truncated: v.take(k_truncated),
            in_reply_to_status_id: v.take(k_in_reply_to_status_id),
            in_reply_to_status_id_str: v.take(k_in_reply_to_status_id_str),
            in_reply_to_user_id: v.take(k_in_reply_to_user_id),
            in_reply_to_user_id_str: v.take(k_in_reply_to_user_id_str),
            in_reply_to_screen_name: v.take(k_in_reply_to_screen_name),
            user: v.take(k_user),
            geo: v.take(k_geo),
            coordinates: v.take(k_coordinates),
            place: v.take(k_place),
            contributors: v.take(k_contributors),
            retweeted_status: v.take(k_retweeted_status),
            retweet_count: v.take(k_retweet_count),
            favorite_count: v.take(k_favorite_count),
            entities: v.take(k_entities),
            favorited: v.take(k_favorited),
            retweeted: v.take(k_retweeted),
            possibly_sensitive: v.take(k_possibly_sensitive),
            lang: v.take(k_lang),
        })
    }
}

fn search_metadata() -> impl Codec<Output = SearchMetadata> + Clone {
    let r = keyed_record::<SearchMetadata>();
    let (r, k_completed_in) = r.field("completed_in", f32(), |x| &x.completed_in);
    let (r, k_max_id) = r.field("max_id", u64(), |x| &x.max_id);
    let (r, k_max_id_str) = r.field("max_id_str", string(), |x| &x.max_id_str);
    let (r, k_next_results) = r.field("next_results", string(), |x| &x.next_results);
    let (r, k_query) = r.field("query", string(), |x| &x.query);
    let (r, k_refresh_url) = r.field("refresh_url", string(), |x| &x.refresh_url);
    let (r, k_count) = r.field("count", u8(), |x| &x.count);
    let (r, k_since_id) = r.field("since_id", u64(), |x| &x.since_id);
    let (r, k_since_id_str) = r.field("since_id_str", string(), |x| &x.since_id_str);
    r.build(move |v| SearchMetadata {
        completed_in: v.take(k_completed_in),
        max_id: v.take(k_max_id),
        max_id_str: v.take(k_max_id_str),
        next_results: v.take(k_next_results),
        query: v.take(k_query),
        refresh_url: v.take(k_refresh_url),
        count: v.take(k_count),
        since_id: v.take(k_since_id),
        since_id_str: v.take(k_since_id_str),
    })
}

pub fn codec() -> impl Codec<Output = Twitter> + Clone {
    let r = keyed_record::<Twitter>();
    let (r, k_statuses) = r.field("statuses", list(recursive(StatusDef)), |x| &x.statuses);
    let (r, k_search_metadata) =
        r.field("search_metadata", search_metadata(), |x| &x.search_metadata);
    r.build(move |v| Twitter {
        statuses: v.take(k_statuses),
        search_metadata: v.take(k_search_metadata),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn girders_matches_serde() {
        let json = crate::data("twitter.json");
        let a: Twitter = serde_json::from_str(&json).unwrap();
        let b = girders_json::decode(&json, &codec()).unwrap();
        assert_eq!(a, b);
        let back = girders_json::encode(&b, &codec());
        assert_eq!(girders_json::decode(&back, &codec()).unwrap(), b);
        assert_eq!(serde_json::from_str::<Twitter>(&back).unwrap(), a);
    }
}
