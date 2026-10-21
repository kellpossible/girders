//! `twitter.json`: a Twitter search API response, string-heavy, wide
//! records (`User` has 40 fields) and one recursive field
//! (`Status.retweeted_status`).
//!
//! Departures from the json-benchmark model, applied to both sides alike:
//! `PrimStr<u64>`/`PrimStr<u32>` (`id_str` etc.) and `Color` are kept as
//! `String`; `empty::Array` (`symbols`) is `[(); 0]`; the `enum_str!`
//! enums are plain fieldless enums.

use girders::prelude::*;

pub type LongId = u64;
pub type ShortId = u32;
pub type Indices = (u8, u8);

#[derive(Debug, Clone, PartialEq)]
pub struct Twitter {
    pub statuses: Vec<Status>,
    pub search_metadata: SearchMetadata,
}

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
pub struct Metadata {
    pub result_type: ResultType,
    pub iso_language_code: LanguageCode,
}

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
pub struct UserEntities {
    pub url: Option<UserUrl>,
    pub description: UserEntitiesDescription,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UserUrl {
    pub urls: Vec<Url>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Url {
    pub url: String,
    pub expanded_url: String,
    pub display_url: String,
    pub indices: Indices,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UserEntitiesDescription {
    pub urls: Vec<Url>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StatusEntities {
    pub hashtags: Vec<Hashtag>,
    pub symbols: [(); 0],
    pub urls: Vec<Url>,
    pub user_mentions: Vec<UserMention>,
    pub media: Option<Vec<Media>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hashtag {
    pub text: String,
    pub indices: Indices,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UserMention {
    pub screen_name: String,
    pub name: String,
    pub id: ShortId,
    pub id_str: String,
    pub indices: Indices,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Media {
    pub id: LongId,
    pub id_str: String,
    pub indices: Indices,
    pub media_url: String,
    pub media_url_https: String,
    pub url: String,
    pub display_url: String,
    pub expanded_url: String,
    pub media_type: String,
    pub sizes: Sizes,
    pub source_status_id: Option<LongId>,
    pub source_status_id_str: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sizes {
    pub medium: Size,
    pub small: Size,
    pub thumb: Size,
    pub large: Size,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Size {
    pub w: u16,
    pub h: u16,
    pub resize: Resize,
}

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Resize {
    Fit,
    Crop,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LanguageCode {
    Cn,
    En,
    Es,
    It,
    Ja,
    Zh,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResultType {
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
    try_convert(
        list(null()),
        |v: Vec<()>| if v.is_empty() { Ok([]) } else { Err("expected an empty array") },
        |_: &[(); 0]| Vec::new(),
    )
}

fn metadata() -> impl Codec<Output = Metadata> + Clone {
    record((
        field("result_type", result_type(), |m: &Metadata| &m.result_type),
        field("iso_language_code", language_code(), |m: &Metadata| &m.iso_language_code),
    ))
    .build(|(result_type, iso_language_code)| Metadata { result_type, iso_language_code })
}

fn url() -> impl Codec<Output = Url> + Clone {
    record((
        field("url", string(), |u: &Url| &u.url),
        field("expanded_url", string(), |u: &Url| &u.expanded_url),
        field("display_url", string(), |u: &Url| &u.display_url),
        field("indices", indices(), |u: &Url| &u.indices),
    ))
    .build(|(url, expanded_url, display_url, indices)| Url { url, expanded_url, display_url, indices })
}

fn user_url() -> impl Codec<Output = UserUrl> + Clone {
    record((field("urls", list(url()), |u: &UserUrl| &u.urls),)).build(|(urls,)| UserUrl { urls })
}

fn user_entities_description() -> impl Codec<Output = UserEntitiesDescription> + Clone {
    record((field("urls", list(url()), |u: &UserEntitiesDescription| &u.urls),))
        .build(|(urls,)| UserEntitiesDescription { urls })
}

fn user_entities() -> impl Codec<Output = UserEntities> + Clone {
    record((
        optional_field("url", user_url(), |u: &UserEntities| &u.url),
        field("description", user_entities_description(), |u: &UserEntities| &u.description),
    ))
    .build(|(url, description)| UserEntities { url, description })
}

fn user() -> impl Codec<Output = User> + Clone {
    record((
        (
            field("id", u32(), |u: &User| &u.id),
            field("id_str", string(), |u: &User| &u.id_str),
            field("name", string(), |u: &User| &u.name),
            field("screen_name", string(), |u: &User| &u.screen_name),
            field("location", string(), |u: &User| &u.location),
            field("description", string(), |u: &User| &u.description),
            optional_field("url", string(), |u: &User| &u.url),
            field("entities", user_entities(), |u: &User| &u.entities),
            field("protected", bool(), |u: &User| &u.protected),
            field("followers_count", u32(), |u: &User| &u.followers_count),
            field("friends_count", u32(), |u: &User| &u.friends_count),
            field("listed_count", u32(), |u: &User| &u.listed_count),
            field("created_at", string(), |u: &User| &u.created_at),
            field("favourites_count", u32(), |u: &User| &u.favourites_count),
            optional_field("utc_offset", i32(), |u: &User| &u.utc_offset),
            optional_field("time_zone", string(), |u: &User| &u.time_zone),
        ),
        (
            field("geo_enabled", bool(), |u: &User| &u.geo_enabled),
            field("verified", bool(), |u: &User| &u.verified),
            field("statuses_count", u32(), |u: &User| &u.statuses_count),
            field("lang", language_code(), |u: &User| &u.lang),
            field("contributors_enabled", bool(), |u: &User| &u.contributors_enabled),
            field("is_translator", bool(), |u: &User| &u.is_translator),
            field("is_translation_enabled", bool(), |u: &User| &u.is_translation_enabled),
            field("profile_background_color", string(), |u: &User| &u.profile_background_color),
            field("profile_background_image_url", string(), |u: &User| &u.profile_background_image_url),
            field("profile_background_image_url_https", string(), |u: &User| {
                &u.profile_background_image_url_https
            }),
            field("profile_background_tile", bool(), |u: &User| &u.profile_background_tile),
            field("profile_image_url", string(), |u: &User| &u.profile_image_url),
            field("profile_image_url_https", string(), |u: &User| &u.profile_image_url_https),
            optional_field("profile_banner_url", string(), |u: &User| &u.profile_banner_url),
            field("profile_link_color", string(), |u: &User| &u.profile_link_color),
            field("profile_sidebar_border_color", string(), |u: &User| &u.profile_sidebar_border_color),
        ),
        (
            field("profile_sidebar_fill_color", string(), |u: &User| &u.profile_sidebar_fill_color),
            field("profile_text_color", string(), |u: &User| &u.profile_text_color),
            field("profile_use_background_image", bool(), |u: &User| &u.profile_use_background_image),
            field("default_profile", bool(), |u: &User| &u.default_profile),
            field("default_profile_image", bool(), |u: &User| &u.default_profile_image),
            field("following", bool(), |u: &User| &u.following),
            field("follow_request_sent", bool(), |u: &User| &u.follow_request_sent),
            field("notifications", bool(), |u: &User| &u.notifications),
        ),
    ))
    .build(
        |(
            (
                id,
                id_str,
                name,
                screen_name,
                location,
                description,
                url,
                entities,
                protected,
                followers_count,
                friends_count,
                listed_count,
                created_at,
                favourites_count,
                utc_offset,
                time_zone,
            ),
            (
                geo_enabled,
                verified,
                statuses_count,
                lang,
                contributors_enabled,
                is_translator,
                is_translation_enabled,
                profile_background_color,
                profile_background_image_url,
                profile_background_image_url_https,
                profile_background_tile,
                profile_image_url,
                profile_image_url_https,
                profile_banner_url,
                profile_link_color,
                profile_sidebar_border_color,
            ),
            (
                profile_sidebar_fill_color,
                profile_text_color,
                profile_use_background_image,
                default_profile,
                default_profile_image,
                following,
                follow_request_sent,
                notifications,
            ),
        )| User {
            id,
            id_str,
            name,
            screen_name,
            location,
            description,
            url,
            entities,
            protected,
            followers_count,
            friends_count,
            listed_count,
            created_at,
            favourites_count,
            utc_offset,
            time_zone,
            geo_enabled,
            verified,
            statuses_count,
            lang,
            contributors_enabled,
            is_translator,
            is_translation_enabled,
            profile_background_color,
            profile_background_image_url,
            profile_background_image_url_https,
            profile_background_tile,
            profile_image_url,
            profile_image_url_https,
            profile_banner_url,
            profile_link_color,
            profile_sidebar_border_color,
            profile_sidebar_fill_color,
            profile_text_color,
            profile_use_background_image,
            default_profile,
            default_profile_image,
            following,
            follow_request_sent,
            notifications,
        },
    )
}

fn hashtag() -> impl Codec<Output = Hashtag> + Clone {
    record((
        field("text", string(), |h: &Hashtag| &h.text),
        field("indices", indices(), |h: &Hashtag| &h.indices),
    ))
    .build(|(text, indices)| Hashtag { text, indices })
}

fn user_mention() -> impl Codec<Output = UserMention> + Clone {
    record((
        field("screen_name", string(), |m: &UserMention| &m.screen_name),
        field("name", string(), |m: &UserMention| &m.name),
        field("id", u32(), |m: &UserMention| &m.id),
        field("id_str", string(), |m: &UserMention| &m.id_str),
        field("indices", indices(), |m: &UserMention| &m.indices),
    ))
    .build(|(screen_name, name, id, id_str, indices)| UserMention { screen_name, name, id, id_str, indices })
}

fn size() -> impl Codec<Output = Size> + Clone {
    record((
        field("w", u16(), |s: &Size| &s.w),
        field("h", u16(), |s: &Size| &s.h),
        field("resize", resize(), |s: &Size| &s.resize),
    ))
    .build(|(w, h, resize)| Size { w, h, resize })
}

fn sizes() -> impl Codec<Output = Sizes> + Clone {
    record((
        field("medium", size(), |s: &Sizes| &s.medium),
        field("small", size(), |s: &Sizes| &s.small),
        field("thumb", size(), |s: &Sizes| &s.thumb),
        field("large", size(), |s: &Sizes| &s.large),
    ))
    .build(|(medium, small, thumb, large)| Sizes { medium, small, thumb, large })
}

fn media() -> impl Codec<Output = Media> + Clone {
    record((
        field("id", u64(), |m: &Media| &m.id),
        field("id_str", string(), |m: &Media| &m.id_str),
        field("indices", indices(), |m: &Media| &m.indices),
        field("media_url", string(), |m: &Media| &m.media_url),
        field("media_url_https", string(), |m: &Media| &m.media_url_https),
        field("url", string(), |m: &Media| &m.url),
        field("display_url", string(), |m: &Media| &m.display_url),
        field("expanded_url", string(), |m: &Media| &m.expanded_url),
        field("type", string(), |m: &Media| &m.media_type),
        field("sizes", sizes(), |m: &Media| &m.sizes),
        optional_field("source_status_id", u64(), |m: &Media| &m.source_status_id),
        optional_field("source_status_id_str", string(), |m: &Media| &m.source_status_id_str),
    ))
    .build(
        |(
            id,
            id_str,
            indices,
            media_url,
            media_url_https,
            url,
            display_url,
            expanded_url,
            media_type,
            sizes,
            source_status_id,
            source_status_id_str,
        )| Media {
            id,
            id_str,
            indices,
            media_url,
            media_url_https,
            url,
            display_url,
            expanded_url,
            media_type,
            sizes,
            source_status_id,
            source_status_id_str,
        },
    )
}

fn status_entities() -> impl Codec<Output = StatusEntities> + Clone {
    record((
        field("hashtags", list(hashtag()), |e: &StatusEntities| &e.hashtags),
        field("symbols", empty_array(), |e: &StatusEntities| &e.symbols),
        field("urls", list(url()), |e: &StatusEntities| &e.urls),
        field("user_mentions", list(user_mention()), |e: &StatusEntities| &e.user_mentions),
        optional_field("media", list(media()), |e: &StatusEntities| &e.media),
    ))
    .build(|(hashtags, symbols, urls, user_mentions, media)| StatusEntities {
        hashtags,
        symbols,
        urls,
        user_mentions,
        media,
    })
}

/// `Status` is recursive through `retweeted_status`.
#[derive(Debug, Clone, Copy)]
struct StatusDef;

impl Recursive for StatusDef {
    type Output = Status;
    fn codec(&self) -> impl Codec<Output = Status> {
        record((
            (
                field("metadata", metadata(), |s: &Status| &s.metadata),
                field("created_at", string(), |s: &Status| &s.created_at),
                field("id", u64(), |s: &Status| &s.id),
                field("id_str", string(), |s: &Status| &s.id_str),
                field("text", string(), |s: &Status| &s.text),
                field("source", string(), |s: &Status| &s.source),
                field("truncated", bool(), |s: &Status| &s.truncated),
                optional_field("in_reply_to_status_id", u64(), |s: &Status| &s.in_reply_to_status_id),
                optional_field("in_reply_to_status_id_str", string(), |s: &Status| &s.in_reply_to_status_id_str),
                optional_field("in_reply_to_user_id", u32(), |s: &Status| &s.in_reply_to_user_id),
                optional_field("in_reply_to_user_id_str", string(), |s: &Status| &s.in_reply_to_user_id_str),
                optional_field("in_reply_to_screen_name", string(), |s: &Status| &s.in_reply_to_screen_name),
                field("user", user(), |s: &Status| &s.user),
                field("geo", null(), |s: &Status| &s.geo),
                field("coordinates", null(), |s: &Status| &s.coordinates),
                field("place", null(), |s: &Status| &s.place),
            ),
            (
                field("contributors", null(), |s: &Status| &s.contributors),
                optional_field("retweeted_status", boxed(recursive(StatusDef)), |s: &Status| &s.retweeted_status),
                field("retweet_count", u32(), |s: &Status| &s.retweet_count),
                field("favorite_count", u32(), |s: &Status| &s.favorite_count),
                field("entities", status_entities(), |s: &Status| &s.entities),
                field("favorited", bool(), |s: &Status| &s.favorited),
                field("retweeted", bool(), |s: &Status| &s.retweeted),
                optional_field("possibly_sensitive", bool(), |s: &Status| &s.possibly_sensitive),
                field("lang", language_code(), |s: &Status| &s.lang),
            ),
        ))
        .build(
            |(
                (
                    metadata,
                    created_at,
                    id,
                    id_str,
                    text,
                    source,
                    truncated,
                    in_reply_to_status_id,
                    in_reply_to_status_id_str,
                    in_reply_to_user_id,
                    in_reply_to_user_id_str,
                    in_reply_to_screen_name,
                    user,
                    geo,
                    coordinates,
                    place,
                ),
                (
                    contributors,
                    retweeted_status,
                    retweet_count,
                    favorite_count,
                    entities,
                    favorited,
                    retweeted,
                    possibly_sensitive,
                    lang,
                ),
            )| Status {
                metadata,
                created_at,
                id,
                id_str,
                text,
                source,
                truncated,
                in_reply_to_status_id,
                in_reply_to_status_id_str,
                in_reply_to_user_id,
                in_reply_to_user_id_str,
                in_reply_to_screen_name,
                user,
                geo,
                coordinates,
                place,
                contributors,
                retweeted_status,
                retweet_count,
                favorite_count,
                entities,
                favorited,
                retweeted,
                possibly_sensitive,
                lang,
            },
        )
    }
}

fn search_metadata() -> impl Codec<Output = SearchMetadata> + Clone {
    record((
        field("completed_in", f32(), |m: &SearchMetadata| &m.completed_in),
        field("max_id", u64(), |m: &SearchMetadata| &m.max_id),
        field("max_id_str", string(), |m: &SearchMetadata| &m.max_id_str),
        field("next_results", string(), |m: &SearchMetadata| &m.next_results),
        field("query", string(), |m: &SearchMetadata| &m.query),
        field("refresh_url", string(), |m: &SearchMetadata| &m.refresh_url),
        field("count", u8(), |m: &SearchMetadata| &m.count),
        field("since_id", u64(), |m: &SearchMetadata| &m.since_id),
        field("since_id_str", string(), |m: &SearchMetadata| &m.since_id_str),
    ))
    .build(
        |(completed_in, max_id, max_id_str, next_results, query, refresh_url, count, since_id, since_id_str)| {
            SearchMetadata {
                completed_in,
                max_id,
                max_id_str,
                next_results,
                query,
                refresh_url,
                count,
                since_id,
                since_id_str,
            }
        },
    )
}

pub fn codec() -> impl Codec<Output = Twitter> + Clone {
    record((
        field("statuses", list(recursive(StatusDef)), |t: &Twitter| &t.statuses),
        field("search_metadata", search_metadata(), |t: &Twitter| &t.search_metadata),
    ))
    .build(|(statuses, search_metadata)| Twitter { statuses, search_metadata })
}

