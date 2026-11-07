//! Client LM Studio / API compatible OpenAI (texte + vision), 100 % local. v0.1.0
use crate::cfg::Config;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

pub struct Llm {
    pub url: String,
    pub model: String,
    pub key: String,
    pub timeout: u64,
    pub no_think: bool,
    pub thumb_px: u32,
}

impl Llm {
    pub fn new(c: &Config) -> Llm {
        Llm { url: c.lm_url.trim_end_matches('/').to_string(), model: c.lm_model.clone(), key: c.lm_key.clone(), timeout: c.lm_timeout.max(5), no_think: c.no_think, thumb_px: c.thumb_px.clamp(128, 2048) }
    }

    fn agent(&self) -> ureq::Agent {
        ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(5)).timeout(Duration::from_secs(self.timeout)).build()
    }

    fn req(&self, method: &str, path: &str) -> ureq::Request {
        let r = self.agent().request(method, &format!("{}{}", self.url, path));
        if self.key.is_empty() { r } else { r.set("Authorization", &format!("Bearer {}", self.key)) }
    }

    /// Modèles chargés (sert aussi de test de connexion).
    pub fn models(&self) -> Result<Vec<String>, String> {
        let v: Value = self.req("GET", "/models").call().map_err(|e| explain(&e))?.into_json().map_err(|e| e.to_string())?;
        Ok(v["data"].as_array().map(|a| a.iter().filter_map(|m| m["id"].as_str().map(String::from)).collect()).unwrap_or_default())
    }

    /// Une requête de discussion ; `images` = data-URI JPEG/PNG.
    pub fn chat(&self, system: &str, user: &str, images: &[String], max_tokens: u32) -> Result<String, String> {
        let mut text = user.to_string();
        if self.no_think {
            text.push_str(" /no_think");
        }
        let content = if images.is_empty() {
            json!(text)
        } else {
            let mut parts = vec![json!({"type":"text","text":text})];
            parts.extend(images.iter().map(|u| json!({"type":"image_url","image_url":{"url":u}})));
            json!(parts)
        };
        let mut body = json!({"messages":[{"role":"system","content":system},{"role":"user","content":content}],"temperature":0.1,"max_tokens":max_tokens});
        if !self.model.is_empty() {
            body["model"] = json!(self.model);
        }
        let v: Value = self.req("POST", "/chat/completions").send_json(body).map_err(|e| explain(&e))?.into_json().map_err(|e| e.to_string())?;
        let s = v["choices"][0]["message"]["content"].as_str().ok_or("réponse vide du modèle")?;
        Ok(strip_think(s))
    }

    /// Comme `chat`, mais renvoie le premier objet JSON de la réponse.
    pub fn chat_json(&self, system: &str, user: &str, images: &[String], max_tokens: u32) -> Result<Value, String> {
        let s = self.chat(system, user, images, max_tokens)?;
        extract_json(&s).ok_or_else(|| format!("réponse non JSON : {}", s.chars().take(120).collect::<String>()))
    }
}

fn explain(e: &ureq::Error) -> String {
    match e {
        ureq::Error::Status(c, r) => format!("HTTP {c} : {}", r.status_text()),
        ureq::Error::Transport(t) => format!("serveur injoignable ({t}). LM Studio est-il lancé, serveur local activé ?"),
    }
}

pub fn strip_think(s: &str) -> String {
    let mut out = s.to_string();
    while let (Some(a), Some(b)) = (out.find("<think>"), out.find("</think>")) {
        if b > a {
            out.replace_range(a..b + 8, "");
        } else {
            break;
        }
    }
    if let Some(b) = out.find("</think>") {
        out = out[b + 8..].to_string();
    }
    out.trim().to_string()
}

/// Premier objet JSON équilibré d'un texte (tolère les ```json … ```).
pub fn extract_json(s: &str) -> Option<Value> {
    let b = s.as_bytes();
    let start = s.find('{')?;
    let (mut depth, mut in_str, mut esc) = (0i32, false, false);
    for (i, &c) in b.iter().enumerate().skip(start) {
        if in_str {
            if esc { esc = false } else if c == b'\\' { esc = true } else if c == b'"' { in_str = false }
            continue;
        }
        match c {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return serde_json::from_str(&s[start..=i]).ok();
                }
            }
            _ => {}
        }
    }
    None
}

pub fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut o = String::with_capacity(data.len() * 4 / 3 + 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        o.push(T[(n >> 18) as usize & 63] as char);
        o.push(T[(n >> 12) as usize & 63] as char);
        o.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        o.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    o
}

/// Miniature JPEG (côté max `px`) en data-URI pour l'IA vision.
pub fn thumb_uri(p: &Path, px: u32) -> Result<String, String> {
    let img = image::open(p).map_err(|e| e.to_string())?;
    let t = img.thumbnail(px, px).to_rgb8();
    let mut buf = vec![];
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 80).encode_image(&t).map_err(|e| e.to_string())?;
    Ok(format!("data:image/jpeg;base64,{}", b64(&buf)))
}

/// Classe un fichier dans une catégorie utilisateur. Photos : vision ; texte : début du fichier ; sinon le nom seul.
pub fn classify(llm: &Llm, f: &crate::scan::Facts, cats: &[String]) -> Result<(String, String, f32), String> {
    let list = cats.join(", ");
    let sys = format!("Tu classes des fichiers. Réponds UNIQUEMENT par un objet JSON : {{\"categorie\":\"<une valeur EXACTE de la liste>\",\"titre\":\"<description très courte, 2 à 5 mots>\",\"confiance\":<0 à 1>}}. Liste des catégories : {list}.");
    let mut images = vec![];
    let mut user = format!("Nom du fichier : {}.", f.name());
    match f.kind() {
        crate::scan::Kind::Photo => images.push(thumb_uri(&f.path, llm.thumb_px)?),
        crate::scan::Kind::Doc if matches!(f.ext.as_str(), "txt" | "md" | "csv" | "html" | "htm") => {
            let mut buf = vec![0u8; 3000];
            use std::io::Read;
            let n = std::fs::File::open(&f.path).and_then(|mut x| x.read(&mut buf)).map_err(|e| e.to_string())?;
            user.push_str(&format!(" Début du contenu :\n{}", String::from_utf8_lossy(&buf[..n])));
        }
        _ => {}
    }
    let v = llm.chat_json(&sys, &user, &images, 200)?;
    let c = v["categorie"].as_str().unwrap_or("").to_string();
    // La catégorie doit appartenir à la liste (cohérence) ; sinon on retombe sur la plus proche ou « Autre ».
    let cat = cats.iter().find(|x| x.eq_ignore_ascii_case(&c)).cloned().unwrap_or_else(|| cats.iter().find(|x| x.eq_ignore_ascii_case("Autre")).cloned().unwrap_or(c));
    Ok((cat, v["titre"].as_str().unwrap_or("").to_string(), v["confiance"].as_f64().unwrap_or(0.5) as f32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::Config;
    #[test]
    fn json_and_think() {
        let s = "<think>hmm {x}</think>\n```json\n{\"a\":\"}\",\"b\":{\"c\":1}}\n```";
        assert_eq!(extract_json(&strip_think(s)).unwrap()["b"]["c"], 1);
    }
    /// Faux serveur LM Studio : répond à /models et /chat/completions avec un contenu fixe.
    fn mock(reply: &'static str) -> String {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for st in l.incoming().flatten() {
                let mut st = st;
                let mut buf = vec![0u8; 65536];
                let n = st.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let body = if req.starts_with("GET /v1/models") {
                    "{\"data\":[{\"id\":\"qwen-test\"}]}".to_string()
                } else {
                    serde_json::json!({"choices":[{"message":{"content":reply}}]}).to_string()
                };
                let _ = write!(st, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
            }
        });
        format!("http://127.0.0.1:{port}/v1")
    }
    #[test]
    fn mock_server_classify_and_command() {
        let url = mock("<think>x</think>{\"categorie\":\"Nature\",\"titre\":\"forêt\",\"confiance\":0.9}");
        let cfg = Config { lm_url: url, ..Default::default() };
        let l = Llm::new(&cfg);
        assert_eq!(l.models().unwrap(), vec!["qwen-test"]);
        let d = std::env::temp_dir().join(format!("af_img_{}.png", std::process::id()));
        image::RgbImage::from_pixel(64, 64, image::Rgb([10, 200, 10])).save(&d).unwrap();
        let f = crate::scan::facts(&d, true).unwrap();
        let (c, t, conf) = classify(&l, &f, &cfg.categories).unwrap();
        assert_eq!((c.as_str(), t.as_str()), ("Nature", "forêt"));
        assert!(conf > 0.8);
        let url2 = mock("{\"action\":\"sort\",\"filter\":{\"kinds\":[\"photo\"]},\"template\":\"P/{année}\",\"explain\":\"ok\"}");
        let c = crate::cmd::parse(&Llm::new(&Config { lm_url: url2, ..Default::default() }), "range mes photos").unwrap();
        assert!(matches!(c.action, crate::cmd::Action::Sort(_)));
        let _ = std::fs::remove_file(d);
        assert!(Llm::new(&Config { lm_url: "http://127.0.0.1:9/v1".into(), ..Default::default() }).models().is_err());
    }
    #[test]
    fn base64() {
        assert_eq!(b64(b"Man"), "TWFu");
        assert_eq!(b64(b"Ma"), "TWE=");
        assert_eq!(b64(b"M"), "TQ==");
    }
}
