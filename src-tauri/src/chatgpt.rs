//! Sign in with ChatGPT : un abonné à ChatGPT Plus ou Pro relie Lumen à son
//! compte, et la traduction des mots, des phrases et le chat puisent dans son
//! forfait au lieu d'une clé payée à part (aperçu d'OpenAI, ouvert le 29 septembre
//! 2026 aux applications open source comme Lumen). Les comptes gratuits peuvent se
//! connecter, mais leur forfait ne sert pas aux autres apps.
//!
//! Connexion : OAuth avec PKCE dans le navigateur du système, retour sur une
//! adresse locale (`http://127.0.0.1:<port>/auth/callback`, seule forme permise
//! par OpenAI), enregistrement du client à la première connexion
//! (`dynamic_agent_client`, puis l'identifiant `oaiapp_…` délivré). Jetons gardés
//! dans `chatgpt.json` du dossier de données (jamais dans la base : ni sauvegarde
//! ni restauration ne les emportent), lisible par l'utilisateur seul.
//!
//! Demandes : API Responses (`store: false`, `stream: true`), sans température ni
//! longueur maximale (refusées), sans message « system » (il devient `instructions`).
//! Rien n'est gardé côté OpenAI : chaque demande porte tout son contexte. Une
//! demande par mot, toujours (jamais une conversation qui s'allonge : chaque mot
//! renverrait tout l'historique et userait le forfait de plus en plus vite).
//!
//! Garde-fou des rafales (les apprenants touchent souvent beaucoup de mots en
//! quelques secondes) : une traduction attend `Pace::settle` avant de partir, et
//! n'est pas envoyée si un autre mot a été touché entre-temps ; au plus
//! `Pace::per_minute` traductions par minute ; limite du forfait atteinte : pause
//! de `Pace::cooldown`, et la traduction se replie sur l'IA de l'appareil.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};
use base64::Engine as _;
use futures_util::StreamExt;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::ai::{Output, Piece};
use crate::i18n::t;
use crate::online::{until, Ask, Check, Config, Unreachable};

/// Nom présenté à OpenAI à l'enregistrement (modifiable par l'apprenant avant d'accepter).
const AGENT_NAME: &str = "Lumen";
const SCOPES: &str = "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
/// Droit d'utiliser le forfait : sans lui, la connexion ne sert qu'à s'identifier.
const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";
const RESOURCE: &str = "https://api.openai.com/v1";
const CALLBACK: &str = "/auth/callback";
/// Ports essayés pour le retour du navigateur (celui de Codex d'abord) ; un autre
/// port libre sinon : seuls le schéma, l'hôte et le chemin doivent rester les mêmes.
const PORTS: [u16; 6] = [1455, 1456, 1457, 1458, 1459, 1460];
/// Page des réglages de ChatGPT où l'on suit et borne ce qu'utilise chaque app.
pub const MANAGE_USAGE: &str = "https://chatgpt.com/#settings/Usage";

/// Adresses d'OpenAI (d'un faux serveur dans les tests).
#[derive(Clone, Debug)]
pub struct Endpoints {
    pub auth: String,
    pub api: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self { auth: "https://auth.openai.com".into(), api: RESOURCE.into() }
    }
}

// ---------- rythme des demandes ----------

/// Rythme des traductions (le chat n'attend pas : on l'envoie quand on l'a écrit).
#[derive(Clone, Copy, Debug)]
pub struct Pace {
    /// attente avant d'envoyer la traduction d'un mot touché : un mot touché
    /// entre-temps la rend caduque, rien n'est parti
    pub settle: Duration,
    /// traductions envoyées au plus par minute (les suivantes attendent leur tour,
    /// et seule la dernière sélection est servie)
    pub per_minute: usize,
    /// pause après la limite du forfait atteinte : la traduction se replie sur
    /// l'IA de l'appareil sans redemander à chaque mot
    pub cooldown: Duration,
}

pub const PACE: Pace = Pace { settle: Duration::from_millis(350), per_minute: 30, cooldown: Duration::from_secs(15 * 60) };

#[cfg(test)]
thread_local! {
    static TEST_PACE: std::cell::Cell<Option<Pace>> = const { std::cell::Cell::new(None) };
}

fn pace() -> Pace {
    #[cfg(test)]
    if let Some(p) = TEST_PACE.with(|x| x.get()) {
        return p;
    }
    PACE
}

// ---------- ce qui est gardé ----------

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct Account {
    /// identifiant délivré à l'enregistrement (`oaiapp_…`), lié à ce compte
    client_id: String,
    email: String,
    subject: String,
    /// dernier jeton d'identité : désigne le compte à la reconnexion (`id_token_hint`)
    id_token: String,
    access_token: String,
    refresh_token: String,
    /// fin de validité du jeton d'accès (secondes Unix)
    expires_at: u64,
    scopes: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct Saved {
    /// identifiant de cet appareil auprès d'OpenAI (`urn:uuid:…`), créé une fois
    host_id: String,
    account: Option<Account>,
    /// demandes du jour (AAAA-MM-JJ), pour que l'apprenant voie ce qu'il dépense
    day: String,
    requests: u32,
}

/// Connexion à ChatGPT de cet appareil, et rythme de ses demandes.
pub struct Vault {
    path: PathBuf,
    ends: Endpoints,
    saved: Mutex<Saved>,
    /// un seul renouvellement à la fois : les jetons de renouvellement ne servent
    /// qu'une fois (un second envoi du même jeton déconnecterait l'apprenant)
    refresh: tokio::sync::Mutex<()>,
    /// envoi des traductions de la dernière minute
    sent: Mutex<VecDeque<Instant>>,
    /// limite du forfait atteinte : pause jusqu'à…
    paused: Mutex<Option<Instant>>,
}

static VAULT: OnceLock<Vault> = OnceLock::new();

/// Au lancement : relit la connexion gardée dans le dossier de données.
pub fn init(data_dir: &Path) {
    let _ = VAULT.set(Vault::at(data_dir.join("chatgpt.json"), Endpoints::default()));
}

fn vault() -> Result<&'static Vault> {
    VAULT.get().ok_or_else(|| anyhow!(t("Connexion à ChatGPT indisponible.", "ChatGPT sign-in unavailable.")))
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

impl Vault {
    pub fn at(path: PathBuf, ends: Endpoints) -> Self {
        let mut saved: Saved = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        if saved.host_id.is_empty() {
            saved.host_id = host_id();
        }
        Self {
            path,
            ends,
            saved: Mutex::new(saved),
            refresh: tokio::sync::Mutex::new(()),
            sent: Mutex::new(VecDeque::new()),
            paused: Mutex::new(None),
        }
    }

    /// Écrit le fichier d'un seul geste, lisible par l'utilisateur seul.
    fn store(&self, s: &Saved) {
        let Ok(bytes) = serde_json::to_vec_pretty(s) else { return };
        let tmp = self.path.with_extension("json.tmp");
        if std::fs::write(&tmp, bytes).is_err() {
            return;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        let _ = std::fs::rename(&tmp, &self.path);
    }

    fn update(&self, f: impl FnOnce(&mut Saved)) {
        let mut s = self.saved.lock();
        f(&mut s);
        self.store(&s);
    }

    fn count_request(&self) {
        let day = today();
        self.update(|s| {
            if s.day != day {
                s.day = day;
                s.requests = 0;
            }
            s.requests += 1;
        });
    }

    fn status(&self) -> Status {
        let s = self.saved.lock();
        let paused = self.paused.lock().and_then(|until| until.checked_duration_since(Instant::now()));
        let requests_today = if s.day == today() { s.requests } else { 0 };
        match &s.account {
            Some(a) => Status {
                signed_in: true,
                email: a.email.clone(),
                plan: a.scopes.split_whitespace().any(|x| x == PLAN_SCOPE),
                expired: a.refresh_token.is_empty(),
                requests_today,
                paused_minutes: paused.map(|d| d.as_secs().div_ceil(60)),
                manage_url: MANAGE_USAGE.into(),
            },
            None => Status { requests_today, manage_url: MANAGE_USAGE.into(), ..Default::default() },
        }
    }
}

/// Ce que l'interface montre (miroir de `ChatGptStatus` dans types.ts).
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub signed_in: bool,
    pub email: String,
    /// le forfait peut servir (abonnement Plus ou Pro, permission accordée)
    pub plan: bool,
    /// connexion à refaire (renouvellement refusé, accès retiré dans ChatGPT)
    pub expired: bool,
    pub requests_today: u32,
    /// limite du forfait atteinte : traductions sur l'appareil pendant encore…
    pub paused_minutes: Option<u64>,
    pub manage_url: String,
}

pub fn status() -> Status {
    VAULT.get().map(Vault::status).unwrap_or_default()
}

/// Connecté, avec le droit d'utiliser le forfait.
pub fn signed_in() -> bool {
    VAULT.get().is_some_and(|v| {
        let st = v.status();
        st.signed_in && st.plan && !st.expired
    })
}

// ---------- hasard et PKCE ----------

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("générateur aléatoire du système");
    b
}

/// Jeton aléatoire en base64url (vérificateur PKCE, `state`, `nonce`).
fn random_token() -> String {
    b64(&random_bytes::<48>())
}

/// `urn:uuid:` d'un UUID v4 : identifiant opaque de cet appareil.
fn host_id() -> String {
    let mut b = random_bytes::<16>();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex::encode(b);
    format!("urn:uuid:{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

fn challenge_of(verifier: &str) -> String {
    b64(&Sha256::digest(verifier.as_bytes()))
}

/// Corps « application/x-www-form-urlencoded ».
fn form(pairs: &[(&str, &str)]) -> String {
    let mut u = reqwest::Url::parse("http://x/").expect("adresse fixe");
    u.query_pairs_mut().extend_pairs(pairs);
    u.query().unwrap_or("").to_string()
}

// ---------- connexion ----------

struct Attempt {
    verifier: String,
    state: String,
    nonce: String,
    redirect: String,
    /// `dynamic_agent_client` (premier enregistrement) ou l'identifiant délivré
    client_id: String,
}

/// Longueur d'adresse qui passe partout : au-delà, l'ouverture par Windows
/// (PowerShell puis ShellExecute) n'est pas garantie entière.
const URL_MAX: usize = 2000;

fn authorize_url(ends: &Endpoints, a: &Attempt, host: &str, hint: Option<(&str, &str)>) -> String {
    let mut u = reqwest::Url::parse(&format!("{}/api/accounts/authorize", ends.auth.trim_end_matches('/'))).expect("adresse d'OpenAI");
    {
        let mut q = u.query_pairs_mut();
        q.append_pair("client_id", &a.client_id);
        if a.client_id == "dynamic_agent_client" {
            q.append_pair("agent_name_hint", AGENT_NAME);
        }
        q.append_pair("ext_agent_host_id", host);
        q.append_pair("response_type", "code");
        q.append_pair("redirect_uri", &a.redirect);
        q.append_pair("scope", SCOPES);
        q.append_pair("resource", RESOURCE);
        q.append_pair("state", &a.state);
        q.append_pair("nonce", &a.nonce);
        q.append_pair("code_challenge_method", "S256");
        q.append_pair("code_challenge", &challenge_of(&a.verifier));
        // le compte à reconnecter, en dernier : seul ce qui est facultatif risquerait d'être coupé
        if let Some((_, email)) = hint.filter(|(_, e)| !e.is_empty()) {
            q.append_pair("login_hint", email);
        }
    }
    // le jeton d'identité désigne le compte plus sûrement que l'adresse e-mail,
    // mais il est long (souvent plus de 1 000 caractères) : seulement si l'adresse reste courte
    if let Some((id_token, _)) = hint.filter(|(i, _)| !i.is_empty()) {
        let mut longer = u.clone();
        longer.query_pairs_mut().append_pair("id_token_hint", id_token);
        if longer.as_str().len() <= URL_MAX {
            return longer.to_string();
        }
    }
    u.to_string()
}

async fn bind_loopback() -> Result<tokio::net::TcpListener> {
    for port in PORTS {
        if let Ok(l) = tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            return Ok(l);
        }
    }
    Ok(tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?)
}

/// Ce que le navigateur rapporte.
#[derive(Debug, PartialEq)]
struct Callback {
    code: String,
    client_id: String,
    scope: String,
}

/// Page montrée dans le navigateur au retour.
fn page(ok: bool) -> String {
    let (title, body) = if ok {
        (t("Lumen est relié à ChatGPT", "Lumen is connected to ChatGPT"), t("Vous pouvez fermer cette page et revenir dans Lumen.", "You can close this page and go back to Lumen."))
    } else {
        (t("La connexion n'a pas abouti", "Sign-in didn't go through"), t("Revenez dans Lumen pour réessayer.", "Go back to Lumen to try again."))
    };
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>Lumen</title>\
         <style>body{{margin:0;min-height:100vh;display:grid;place-items:center;font:16px -apple-system,system-ui,sans-serif;\
         background:radial-gradient(circle at 50% 30%,#fff3d6,#f7efe2 60%);color:#3b2f1e}}\
         @media(prefers-color-scheme:dark){{body{{background:radial-gradient(circle at 50% 30%,#3a2a14,#17120c 60%);color:#f3e6cf}}}}\
         .o{{width:44px;height:44px;border-radius:50%;margin:0 auto 18px;background:radial-gradient(circle,#ffe4a3,#f0a73c);box-shadow:0 0 40px #f6b44f99}}\
         main{{text-align:center;padding:24px}}h1{{font-weight:600;font-size:22px;margin:0 0 8px}}p{{margin:0;opacity:.75}}</style></head>\
         <body><main><div class=\"o\"></div><h1>{title}</h1><p>{body}</p></main></body></html>"
    );
    format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}", html.len())
}

/// Attend le retour du navigateur sur l'adresse locale. `back` est appelé dès
/// qu'il revient, accord ou refus (Lumen repasse devant le navigateur).
async fn wait_callback(listener: tokio::net::TcpListener, state: &str, cancel: &AtomicBool, limit: Duration, back: impl FnOnce()) -> Result<Callback> {
    let deadline = Instant::now() + limit;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(anyhow!("annulé"));
        }
        if Instant::now() > deadline {
            return Err(anyhow!(t("La connexion à ChatGPT a pris trop de temps. Réessayez.", "Signing in to ChatGPT took too long. Try again.")));
        }
        let Ok(accepted) = tokio::time::timeout(Duration::from_millis(300), listener.accept()).await else { continue };
        let Ok((mut sock, _)) = accepted else { continue };
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 64 * 1024 {
            match tokio::time::timeout(Duration::from_secs(5), sock.read(&mut tmp)).await {
                Ok(Ok(n)) if n > 0 => buf.extend_from_slice(&tmp[..n]),
                _ => break,
            }
        }
        let head = String::from_utf8_lossy(&buf).to_string();
        let target = head.split_whitespace().nth(1).unwrap_or("");
        let Ok(url) = reqwest::Url::parse(&format!("http://127.0.0.1{target}")) else { continue };
        if url.path() != CALLBACK {
            // icône de la page, autre demande du navigateur
            let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            continue;
        }
        let q = |k: &str| url.query_pairs().find(|(n, _)| n == k).map(|(_, v)| v.to_string()).unwrap_or_default();
        let ok = q("state") == state && q("error").is_empty() && !q("code").is_empty();
        let _ = sock.write_all(page(ok).as_bytes()).await;
        let _ = sock.shutdown().await;
        back();
        if q("state") != state {
            return Err(anyhow!(t("Réponse de connexion inattendue. Réessayez.", "Unexpected sign-in answer. Try again.")));
        }
        if !q("error").is_empty() {
            return Err(anyhow!(t("Connexion refusée dans ChatGPT.", "Sign-in declined in ChatGPT.")));
        }
        return Ok(Callback { code: q("code"), client_id: q("client_id"), scope: q("scope") });
    }
}

/// Champs utiles du jeton d'identité, vérifiés.
#[derive(Debug, PartialEq)]
struct Identity {
    email: String,
    subject: String,
}

/// Vérifie le jeton d'identité : émetteur, destinataire, validité, `nonce`. Il
/// arrive directement du serveur d'OpenAI par une connexion TLS vérifiée : la
/// norme OpenID Connect (Core 3.1.3.7, point 6) permet alors de s'en remettre à
/// TLS plutôt qu'à la signature (pas de bibliothèque de cryptographie de plus).
fn check_id_token(token: &str, issuer: &str, client_id: &str, nonce: Option<&str>) -> Result<Identity> {
    let bad = || anyhow!(t("Jeton d'identité de ChatGPT illisible.", "Unreadable ChatGPT identity token."));
    let payload = token.split('.').nth(1).ok_or_else(bad)?;
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).map_err(|_| bad())?;
    let c: Value = serde_json::from_slice(&raw).map_err(|_| bad())?;
    let aud_ok = match &c["aud"] {
        Value::String(a) => a == client_id,
        Value::Array(list) => list.iter().any(|a| a.as_str() == Some(client_id)),
        _ => false,
    };
    let fresh = c["exp"].as_u64().is_some_and(|exp| exp + 300 > now_secs());
    let nonce_ok = nonce.is_none_or(|n| c["nonce"].as_str() == Some(n));
    if c["iss"].as_str().map(|s| s.trim_end_matches('/')) != Some(issuer.trim_end_matches('/')) || !aud_ok || !fresh || !nonce_ok {
        return Err(bad());
    }
    Ok(Identity { email: c["email"].as_str().unwrap_or("").to_string(), subject: c["sub"].as_str().unwrap_or("").to_string() })
}

fn client() -> reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(90))
            .pool_idle_timeout(Duration::from_secs(120))
            .build()
            .unwrap_or_default()
    })
    .clone()
}

async fn post_form(url: &str, pairs: &[(&str, &str)]) -> Result<(u16, Value)> {
    let r = client()
        .post(url)
        .header("content-type", "application/x-www-form-urlencoded")
        .header("accept", "application/json")
        .body(form(pairs))
        .send()
        .await
        .map_err(|_| anyhow::Error::new(Unreachable(t("ChatGPT est injoignable. Vérifiez la connexion à Internet.", "ChatGPT can't be reached. Check the Internet connection.").into())))?;
    let status = r.status().as_u16();
    let text = r.text().await.unwrap_or_default();
    Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
}

/// Connexion dans le navigateur. `open` ouvre l'adresse dans le navigateur du
/// système ; `back` ramène Lumen au premier plan quand le navigateur revient ;
/// `cancel` l'abandonne (bouton « Annuler »).
pub async fn sign_in(open: impl FnOnce(&str) -> Result<()>, back: impl FnOnce(), cancel: Arc<AtomicBool>) -> Result<Status> {
    sign_in_with(vault()?, open, back, &cancel, Duration::from_secs(600)).await
}

async fn sign_in_with(v: &Vault, open: impl FnOnce(&str) -> Result<()>, back: impl FnOnce(), cancel: &AtomicBool, limit: Duration) -> Result<Status> {
    let listener = bind_loopback().await?;
    let port = listener.local_addr()?.port();
    let (host, previous) = {
        let s = v.saved.lock();
        (s.host_id.clone(), s.account.clone())
    };
    let attempt = Attempt {
        verifier: random_token(),
        state: random_token(),
        nonce: random_token(),
        redirect: format!("http://127.0.0.1:{port}{CALLBACK}"),
        // reconnexion d'un compte déjà enregistré : son identifiant ; sinon, enregistrement
        client_id: previous.as_ref().map(|a| a.client_id.clone()).filter(|c| !c.is_empty()).unwrap_or_else(|| "dynamic_agent_client".into()),
    };
    let hint = previous.as_ref().map(|a| (a.id_token.as_str(), a.email.as_str()));
    open(&authorize_url(&v.ends, &attempt, &host, hint))?;
    let cb = wait_callback(listener, &attempt.state, cancel, limit, back).await?;
    let client_id = if attempt.client_id == "dynamic_agent_client" {
        if cb.client_id.is_empty() {
            return Err(anyhow!(t("ChatGPT n'a pas terminé l'enregistrement de Lumen. Réessayez.", "ChatGPT didn't finish registering Lumen. Try again.")));
        }
        cb.client_id.clone()
    } else {
        if !cb.client_id.is_empty() && cb.client_id != attempt.client_id {
            return Err(anyhow!(t("Réponse de connexion inattendue. Réessayez.", "Unexpected sign-in answer. Try again.")));
        }
        attempt.client_id.clone()
    };
    let token_url = format!("{}/api/accounts/oauth/token", v.ends.auth.trim_end_matches('/'));
    let (status, body) = post_form(
        &token_url,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", &client_id),
            ("code", &cb.code),
            ("code_verifier", &attempt.verifier),
            ("redirect_uri", &attempt.redirect),
            ("resource", RESOURCE),
        ],
    )
    .await?;
    if status != 200 {
        return Err(anyhow!(t("ChatGPT a refusé la connexion. Réessayez.", "ChatGPT refused the sign-in. Try again.")));
    }
    let id_token = body["id_token"].as_str().unwrap_or("").to_string();
    let who = check_id_token(&id_token, &v.ends.auth, &client_id, Some(&attempt.nonce))?;
    if let Some(prev) = &previous {
        if !prev.subject.is_empty() && prev.subject != who.subject {
            // un autre compte : on repartira d'un enregistrement neuf
            v.update(|s| s.account = None);
            return Err(anyhow!(t("Un autre compte ChatGPT a répondu. Déconnectez-vous d'abord, puis reconnectez-vous.", "A different ChatGPT account answered. Sign out first, then sign in again.")));
        }
    }
    let scopes = body["scope"].as_str().filter(|s| !s.is_empty()).unwrap_or(&cb.scope).to_string();
    let account = Account {
        client_id,
        email: who.email,
        subject: who.subject,
        id_token,
        access_token: body["access_token"].as_str().unwrap_or("").to_string(),
        refresh_token: body["refresh_token"].as_str().unwrap_or("").to_string(),
        expires_at: now_secs() + body["expires_in"].as_u64().unwrap_or(3600),
        scopes,
    };
    v.update(|s| s.account = Some(account));
    *v.paused.lock() = None;
    Ok(v.status())
}

/// Déconnexion : le jeton de renouvellement est révoqué chez OpenAI (au mieux),
/// puis oublié. L'identifiant de l'appareil reste.
pub async fn sign_out() -> Result<Status> {
    let v = vault()?;
    let account = v.saved.lock().account.clone();
    if let Some(a) = account.filter(|a| !a.refresh_token.is_empty()) {
        let discovery = format!("{}/.well-known/openid-configuration", v.ends.auth.trim_end_matches('/'));
        if let Ok(r) = client().get(&discovery).timeout(Duration::from_secs(8)).send().await {
            if let Some(url) = r.json::<Value>().await.ok().and_then(|d| d["revocation_endpoint"].as_str().map(str::to_string)) {
                let _ = post_form(&url, &[("token", &a.refresh_token), ("token_type_hint", "refresh_token"), ("client_id", &a.client_id)]).await;
            }
        }
    }
    v.update(|s| s.account = None);
    *v.paused.lock() = None;
    Ok(v.status())
}

// ---------- jetons ----------

const DEAD_REFRESH: [&str; 6] = ["invalid_grant", "invalid_refresh_token", "token_expired", "refresh_token_expired", "refresh_token_invalidated", "refresh_token_reused"];

fn reconnect() -> anyhow::Error {
    anyhow!(t("La connexion de Lumen à ChatGPT a expiré. Reconnectez-vous dans Réglages › IA.", "Lumen's connection to ChatGPT has expired. Sign in again in Settings › AI."))
}

/// Jeton d'accès valable (renouvelé s'il expire dans moins de deux minutes).
async fn token(v: &Vault) -> Result<String> {
    let fresh = |v: &Vault| -> Result<Option<String>> {
        let s = v.saved.lock();
        let a = s.account.as_ref().ok_or_else(|| anyhow!(t("Connectez-vous à ChatGPT dans Réglages › IA.", "Sign in to ChatGPT in Settings › AI.")))?;
        if a.refresh_token.is_empty() && a.access_token.is_empty() {
            return Err(reconnect());
        }
        Ok((!a.access_token.is_empty() && a.expires_at > now_secs() + 120).then(|| a.access_token.clone()))
    };
    if let Some(tok) = fresh(v)? {
        return Ok(tok);
    }
    let _one = v.refresh.lock().await;
    // renouvelé par une autre demande pendant l'attente ?
    if let Some(tok) = fresh(v)? {
        return Ok(tok);
    }
    refresh_now(v).await
}

/// Renouvelle le jeton d'accès (le jeton de renouvellement change à chaque fois).
async fn refresh_now(v: &Vault) -> Result<String> {
    let Some(a) = v.saved.lock().account.clone() else { return Err(reconnect()) };
    if a.refresh_token.is_empty() {
        return Err(reconnect());
    }
    let token_url = format!("{}/api/accounts/oauth/token", v.ends.auth.trim_end_matches('/'));
    let (status, body) = post_form(&token_url, &[("grant_type", "refresh_token"), ("client_id", &a.client_id), ("refresh_token", &a.refresh_token), ("resource", RESOURCE)]).await?;
    if status == 200 && body["access_token"].is_string() {
        let access = body["access_token"].as_str().unwrap_or("").to_string();
        v.update(|s| {
            if let Some(acc) = s.account.as_mut() {
                acc.access_token = access.clone();
                if let Some(r) = body["refresh_token"].as_str() {
                    acc.refresh_token = r.to_string();
                }
                if let Some(i) = body["id_token"].as_str() {
                    acc.id_token = i.to_string();
                }
                if let Some(sc) = body["scope"].as_str().filter(|x| !x.is_empty()) {
                    acc.scopes = sc.to_string();
                }
                acc.expires_at = now_secs() + body["expires_in"].as_u64().unwrap_or(3600);
            }
        });
        return Ok(access);
    }
    let code = body["error"].as_str().or(body["error"]["code"].as_str()).unwrap_or("");
    if DEAD_REFRESH.contains(&code) || matches!(status, 400 | 401) {
        // connexion à refaire ; on garde le compte pour la proposer avec son adresse
        v.update(|s| {
            if let Some(acc) = s.account.as_mut() {
                acc.access_token.clear();
                acc.refresh_token.clear();
            }
        });
        return Err(reconnect());
    }
    Err(anyhow::Error::new(Unreachable(t("ChatGPT ne répond pas pour le moment. Réessayez dans un instant.", "ChatGPT isn't answering right now. Try again in a moment.").into())))
}

// ---------- la demande ----------

/// Réglages de la réflexion essayés l'un après l'autre si le modèle en refuse un.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Level {
    /// effort « none » (traduction) ou celui du chat, avec un résumé de la réflexion
    Full,
    /// effort « minimal » (traduction), ou sans résumé (chat)
    Alt,
    /// sans réglage de la réflexion
    Plain,
}

impl Level {
    fn next(self) -> Option<Level> {
        match self {
            Level::Full => Some(Level::Alt),
            Level::Alt => Some(Level::Plain),
            Level::Plain => None,
        }
    }
}

fn levels() -> &'static Mutex<std::collections::HashMap<String, Level>> {
    static L: OnceLock<Mutex<std::collections::HashMap<String, Level>>> = OnceLock::new();
    L.get_or_init(Default::default)
}

/// Corps de la demande (API Responses) : les consignes en `instructions`, le reste
/// en `input` ; ni température ni longueur maximale (refusées sur cette route).
fn body(cfg: &Config, ask: &Ask, level: Level) -> Value {
    let mut instructions: Vec<&str> = Vec::new();
    let mut input: Vec<Value> = Vec::new();
    for (role, content) in ask.messages {
        match *role {
            "system" if input.is_empty() => instructions.push(content),
            // une consigne glissée plus loin dans la conversation : message « developer »
            "system" => input.push(json!({ "role": "developer", "content": content })),
            r => input.push(json!({ "role": r, "content": content })),
        }
    }
    let mut b = json!({ "model": cfg.model, "input": input, "stream": true, "store": false });
    if !instructions.is_empty() {
        b["instructions"] = json!(instructions.join("\n\n"));
    }
    match (level, ask.think) {
        (Level::Full, None) => b["reasoning"] = json!({ "effort": "none" }),
        (Level::Alt, None) => b["reasoning"] = json!({ "effort": "minimal" }),
        (Level::Full, Some(e)) => b["reasoning"] = json!({ "effort": e, "summary": "auto" }),
        (Level::Alt, Some(e)) => b["reasoning"] = json!({ "effort": e }),
        (Level::Plain, _) => {}
    }
    b
}

/// Un événement du flux (« data: {…} »).
#[derive(Debug, PartialEq)]
enum Event {
    Text(String),
    Thought(String),
    Done,
    Failed { code: String, message: String },
}

fn parse_event(line: &str) -> Option<Event> {
    let data = line.trim().strip_prefix("data:")?.trim();
    let v: Value = serde_json::from_str(data).ok()?;
    let s = |x: &Value| x.as_str().unwrap_or("").to_string();
    Some(match v["type"].as_str()? {
        "response.output_text.delta" => Event::Text(s(&v["delta"])),
        "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => Event::Thought(s(&v["delta"])),
        // réponse coupée (longueur, filtre) : on garde ce qui est écrit
        "response.completed" | "response.incomplete" => Event::Done,
        "response.failed" => Event::Failed { code: s(&v["response"]["error"]["code"]), message: s(&v["response"]["error"]["message"]) },
        "error" => Event::Failed { code: s(&v["code"]).max(s(&v["error"]["code"])), message: s(&v["message"]).max(s(&v["error"]["message"])) },
        _ => return None,
    })
}

/// Code d'erreur d'une réponse refusée (`error.code`, ou `detail` d'un refus d'admission).
fn error_of(text: &str) -> (String, String, String) {
    let v: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let s = |x: &Value| x.as_str().unwrap_or("").to_string();
    let message = [s(&v["error"]["message"]), s(&v["detail"]), s(&v["message"])].into_iter().find(|m| !m.is_empty()).unwrap_or_else(|| text.chars().take(200).collect());
    (s(&v["error"]["code"]), s(&v["error"]["param"]), message)
}

const USAGE_LIMIT: &str = "subscription_sharing_usage_limit_exceeded";

/// Limite du forfait atteinte : message, et repli de la traduction sur l'appareil.
fn limit_error(minutes: u64) -> anyhow::Error {
    let here = crate::i18n::this_device();
    anyhow::Error::new(Unreachable(tr!(
        "La limite d'utilisation de votre forfait ChatGPT est atteinte pour Lumen. Lumen traduit avec l'IA de {here} pendant {minutes} min ; vous pouvez relever la limite dans ChatGPT, Réglages › Utilisation.",
        "Your ChatGPT plan's usage limit for Lumen has been reached. Lumen translates with {here}'s AI for {minutes} min; you can raise the limit in ChatGPT, Settings › Usage."
    )))
}

/// Une erreur de ChatGPT, dite pour l'apprenant.
fn api_error(cfg: &Config, status: u16, code: &str, param: &str, message: &str) -> anyhow::Error {
    let model = &cfg.model;
    let busy = || anyhow::Error::new(Unreachable(t("ChatGPT ne peut pas répondre pour le moment. Réessayez dans un instant.", "ChatGPT can't answer right now. Try again in a moment.").into()));
    match (status, code) {
        (403, "subscription_sharing_user_not_eligible") => anyhow!(t(
            "Votre compte ChatGPT ne peut pas prêter son forfait à d'autres apps : il faut un abonnement Plus ou Pro (ou l'accord de l'administrateur de votre espace de travail).",
            "Your ChatGPT account can't lend its plan to other apps: it takes a Plus or Pro subscription (or your workspace admin's approval)."
        )),
        (401, _) => reconnect(),
        (403, _) if message.to_lowercase().contains("region") => anyhow!(t("ChatGPT n'accepte pas les demandes venues de ce pays.", "ChatGPT doesn't accept requests from this country.")),
        (403, _) => anyhow!(tr!("ChatGPT refuse cette demande ({code}). Reconnectez-vous dans Réglages › IA.", "ChatGPT refuses this request ({code}). Sign in again in Settings › AI.")),
        (404, _) => anyhow!(tr!(
            "ChatGPT ne propose pas « {model} » à votre compte. Choisissez un autre modèle dans Réglages › IA.",
            "ChatGPT doesn't offer “{model}” to your account. Choose another model in Settings › AI."
        )),
        (400, "subscription_sharing_unsupported_capability") => anyhow!(tr!("ChatGPT refuse un réglage de la demande ({param}).", "ChatGPT refuses a request setting ({param}).")),
        (429, _) | (500..=599, _) => busy(),
        _ => anyhow!(tr!("ChatGPT a répondu : {message}", "ChatGPT answered: {message}")),
    }
}

impl Vault {
    /// Pause en cours après la limite du forfait : minutes restantes.
    fn paused_for(&self) -> Option<u64> {
        let mut p = self.paused.lock();
        match *p {
            Some(until) if until > Instant::now() => Some((until - Instant::now()).as_secs().div_ceil(60)),
            Some(_) => {
                *p = None;
                None
            }
            None => None,
        }
    }

    /// Place dans la minute : `None` si la traduction peut partir (elle est comptée),
    /// sinon le temps avant la prochaine place.
    fn slot(&self, per_minute: usize) -> Option<Duration> {
        let mut sent = self.sent.lock();
        let now = Instant::now();
        while sent.front().is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(60)) {
            sent.pop_front();
        }
        if sent.len() < per_minute {
            sent.push_back(now);
            return None;
        }
        sent.front().map(|t| Duration::from_secs(60).saturating_sub(now.duration_since(*t)))
    }
}

/// Génération au fil de l'eau par le forfait ChatGPT (même forme que `online::run`).
pub async fn run(cfg: &Config, ask: Ask<'_>, halted: impl Fn() -> Result<bool>, on_piece: impl FnMut(Piece) -> bool) -> Result<Output> {
    run_with(vault()?, cfg, ask, halted, on_piece, true).await
}

async fn run_with(v: &Vault, cfg: &Config, ask: Ask<'_>, halted: impl Fn() -> Result<bool>, mut on_piece: impl FnMut(Piece) -> bool, paced: bool) -> Result<Output> {
    let mut out = Output { thought: String::new(), answer: String::new(), stopped: false };
    let p = pace();
    if ask.exact && paced {
        if let Some(minutes) = v.paused_for() {
            return Err(limit_error(minutes));
        }
        // un mot touché juste après celui-ci l'emporte : rien n'est parti
        if until(tokio::time::sleep(p.settle), &halted).await?.is_none() {
            out.stopped = true;
            return Ok(out);
        }
        while let Some(wait) = v.slot(p.per_minute) {
            if until(tokio::time::sleep(wait.min(Duration::from_millis(250))), &halted).await?.is_none() {
                out.stopped = true;
                return Ok(out);
            }
        }
    }
    let base = cfg.base.trim_end_matches('/').to_string();
    let memo = format!("{}|{}", cfg.model, ask.think.is_some());
    let mut level = levels().lock().get(&memo).copied().unwrap_or(Level::Full);
    let first = level;
    let (mut refreshed, mut attempt) = (false, 0u32);
    let res = loop {
        let token = token(v).await?;
        // un autre mot touché pendant l'attente : rien ne part, rien n'est compté
        if halted()? {
            out.stopped = true;
            return Ok(out);
        }
        let payload = serde_json::to_vec(&body(cfg, &ask, level))?;
        v.count_request();
        let send = client()
            .post(format!("{base}/responses"))
            .bearer_auth(&token)
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .body(payload)
            .send();
        let Some(res) = until(send, &halted).await? else {
            out.stopped = true;
            return Ok(out);
        };
        let res = match res {
            Ok(r) => r,
            Err(e) if attempt < 1 && (e.is_connect() || e.is_timeout()) => {
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(800)).await;
                continue;
            }
            Err(_) => return Err(anyhow::Error::new(Unreachable(t("ChatGPT est injoignable. Vérifiez la connexion à Internet.", "ChatGPT can't be reached. Check the Internet connection.").into()))),
        };
        let status = res.status().as_u16();
        if status == 200 {
            break res;
        }
        let pause_hint = res.headers().get("retry-after").and_then(|h| h.to_str().ok()).and_then(|s| s.trim().parse::<f64>().ok()).filter(|s| s.is_finite() && *s >= 0.0).map(Duration::from_secs_f64);
        let text = res.text().await.unwrap_or_default();
        let (code, param, message) = error_of(&text);
        // jeton refusé : renouvelé une fois
        if status == 401 && !refreshed {
            refreshed = true;
            let _one = v.refresh.lock().await;
            refresh_now(v).await?;
            continue;
        }
        // réglage refusé par ce modèle : on recommence avec moins
        if status == 400 && code != "subscription_sharing_user_not_eligible" {
            if let Some(next) = level.next() {
                level = next;
                continue;
            }
        }
        if status == 429 && code == USAGE_LIMIT {
            *v.paused.lock() = Some(Instant::now() + p.cooldown);
            return Err(limit_error(p.cooldown.as_secs().div_ceil(60)));
        }
        // trop de demandes à la fois, serveur débordé : une ou deux attentes courtes
        let pause = pause_hint.unwrap_or(Duration::from_millis(1500 << attempt));
        if (status == 429 || status >= 500) && attempt < 2 && pause <= Duration::from_secs(8) {
            attempt += 1;
            if until(tokio::time::sleep(pause), &halted).await?.is_none() {
                out.stopped = true;
                return Ok(out);
            }
            continue;
        }
        return Err(api_error(cfg, status, &code, &param, &message));
    };
    if level != first {
        levels().lock().insert(memo, level);
    }

    let mut stream = res.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    let mut completed = false;
    'read: loop {
        let Some(next) = until(stream.next(), &halted).await? else {
            out.stopped = true;
            break;
        };
        let Some(chunk) = next else { break };
        let Ok(chunk) = chunk else { break };
        buf.extend_from_slice(&chunk);
        while let Some(i) = buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = buf.drain(..=i).collect();
            match parse_event(&String::from_utf8_lossy(&line)) {
                Some(Event::Thought(d)) if out.answer.is_empty() && !d.is_empty() => {
                    out.thought.push_str(&d);
                    if !on_piece(Piece::Thought(&d)) {
                        break 'read;
                    }
                }
                Some(Event::Text(d)) => {
                    if !crate::ai::answer_piece(&mut out, &d, &mut on_piece) {
                        break 'read;
                    }
                }
                Some(Event::Done) => {
                    completed = true;
                    break 'read;
                }
                Some(Event::Failed { code, message }) => {
                    if code == USAGE_LIMIT {
                        *v.paused.lock() = Some(Instant::now() + p.cooldown);
                        if out.answer.is_empty() {
                            return Err(limit_error(p.cooldown.as_secs().div_ceil(60)));
                        }
                    }
                    if out.answer.is_empty() {
                        return Err(api_error(cfg, if code.is_empty() { 500 } else { 0 }, &code, "", &message));
                    }
                    // réponse déjà commencée : on garde ce qui est écrit
                    break 'read;
                }
                _ => {}
            }
        }
    }
    // flux coupé avant la fin, sans un mot : rien de sûr à montrer
    if !completed && !out.stopped && out.answer.is_empty() {
        return Err(anyhow::Error::new(Unreachable(t("La réponse de ChatGPT s'est interrompue. Réessayez.", "ChatGPT's answer was cut off. Try again.").into())));
    }
    out.thought = out.thought.trim().to_string();
    Ok(out)
}

// ---------- modèles ----------

/// Modèles proposés à ce compte, dans l'ordre du serveur (`visibility` « list »).
fn models_of(v: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(list) = v["models"].as_array() {
        for m in list {
            let visible = m["visibility"].as_str().is_none_or(|x| x == "list");
            if let Some(slug) = m["slug"].as_str().or(m["id"].as_str()).filter(|_| visible) {
                out.push(slug.to_string());
            }
        }
    } else if let Some(list) = v["data"].as_array() {
        out.extend(list.iter().filter_map(|m| m["id"].as_str().map(str::to_string)));
    }
    out.dedup();
    out
}

/// Conseillé : le plus récent des « mini » (rapide, léger pour le forfait), sinon le premier.
fn pick(ids: &[String]) -> Option<String> {
    crate::online::newest(ids, "gpt-", "mini").or_else(|| ids.iter().find(|x| x.contains("mini")).cloned()).or_else(|| ids.first().cloned())
}

async fn list_models(v: &Vault) -> Result<Vec<String>> {
    let token = token(v).await?;
    let r = client()
        .get(format!("{}/models", v.ends.api.trim_end_matches('/')))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|_| anyhow::Error::new(Unreachable(t("ChatGPT est injoignable. Vérifiez la connexion à Internet.", "ChatGPT can't be reached. Check the Internet connection.").into())))?;
    let status = r.status().as_u16();
    let text = r.text().await.unwrap_or_default();
    if status != 200 {
        let (code, param, message) = error_of(&text);
        let cfg = config_for(v, "");
        return Err(api_error(&cfg, status, &code, &param, &message));
    }
    Ok(models_of(&serde_json::from_str(&text).unwrap_or(Value::Null)))
}

fn config_for(v: &Vault, model: &str) -> Config {
    let p = crate::online::find("chatgpt").expect("fournisseur ChatGPT");
    Config { provider: p, name: p.name.into(), base: v.ends.api.clone(), key: String::new(), model: model.into() }
}

/// Vérifie la connexion : modèles proposés, puis une toute petite question (le
/// forfait peut servir, et en combien de temps le modèle répond).
pub async fn check(model: &str) -> Result<Check> {
    check_with(vault()?, model).await
}

async fn check_with(v: &Vault, model: &str) -> Result<Check> {
    let models = list_models(v).await?;
    let model = model.trim();
    let chosen = if !model.is_empty() && (models.is_empty() || models.iter().any(|m| m == model)) { model.to_string() } else { pick(&models).unwrap_or_default() };
    if chosen.is_empty() {
        return Err(anyhow!(t("ChatGPT ne propose aucun modèle à ce compte.", "ChatGPT offers no model to this account.")));
    }
    let cfg = config_for(v, &chosen);
    let start = Instant::now();
    let messages = [("user", "Reply with the single word: OK".to_string())];
    run_with(v, &cfg, Ask { messages: &messages, max_tokens: 16, think: None, exact: true }, || Ok(false), |_| true, false).await?;
    Ok(Check { models, model: chosen, ms: start.elapsed().as_millis() as u64 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, AtomicUsize};

    fn jwt(claims: Value) -> String {
        format!("{}.{}.sig", b64(br#"{"alg":"RS256"}"#), b64(claims.to_string().as_bytes()))
    }

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lumen-chatgpt-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("chatgpt.json")
    }

    #[test]
    fn host_ids_and_pkce_follow_the_rules() {
        let h = host_id();
        assert!(h.starts_with("urn:uuid:") && h.len() == 45, "{h}");
        assert_eq!(&h[23..24], "4", "UUID v4 : {h}");
        assert_ne!(host_id(), h);
        // exemple de la RFC 7636
        assert_eq!(challenge_of("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        let tok = random_token();
        assert_eq!(tok.len(), 64);
        assert!(tok.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn authorize_url_has_everything() {
        let a = Attempt { verifier: "v".into(), state: "s1".into(), nonce: "n1".into(), redirect: "http://127.0.0.1:1455/auth/callback".into(), client_id: "dynamic_agent_client".into() };
        let u = reqwest::Url::parse(&authorize_url(&Endpoints::default(), &a, "urn:uuid:x", None)).unwrap();
        assert_eq!(u.as_str().split('?').next(), Some("https://auth.openai.com/api/accounts/authorize"));
        let q: std::collections::HashMap<String, String> = u.query_pairs().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        assert_eq!(q["client_id"], "dynamic_agent_client");
        assert_eq!(q["agent_name_hint"], "Lumen");
        assert_eq!(q["scope"], SCOPES);
        assert_eq!(q["resource"], RESOURCE);
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["code_challenge"], challenge_of("v"));
        assert_eq!(q["redirect_uri"], "http://127.0.0.1:1455/auth/callback");
        assert!(!q.contains_key("id_token_hint"));
        // reconnexion : l'identifiant délivré, et le compte désigné
        let a = Attempt { client_id: "oaiapp_1".into(), ..a };
        let u = reqwest::Url::parse(&authorize_url(&Endpoints::default(), &a, "urn:uuid:x", Some(("tok", "lea@example.com")))).unwrap();
        let q: std::collections::HashMap<String, String> = u.query_pairs().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        assert_eq!((q["client_id"].as_str(), q["id_token_hint"].as_str(), q["login_hint"].as_str()), ("oaiapp_1", "tok", "lea@example.com"));
        assert!(!q.contains_key("agent_name_hint"));
        // un vrai jeton d'identité est long : l'adresse reste ouvrable partout (Windows),
        // l'adresse e-mail désigne alors le compte
        let long = "x".repeat(3000);
        let url = authorize_url(&Endpoints::default(), &a, "urn:uuid:x", Some((&long, "lea@example.com")));
        assert!(url.len() <= URL_MAX, "{}", url.len());
        let q: std::collections::HashMap<String, String> = reqwest::Url::parse(&url).unwrap().query_pairs().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        assert!(!q.contains_key("id_token_hint"));
        assert_eq!((q["login_hint"].as_str(), q["code_challenge"].as_str()), ("lea@example.com", challenge_of("v").as_str()));
    }

    #[test]
    fn id_tokens_are_checked() {
        let exp = now_secs() + 3600;
        let tok = jwt(json!({ "iss": "https://auth.openai.com", "aud": "oaiapp_1", "exp": exp, "nonce": "n", "sub": "user-1", "email": "lea@example.com" }));
        let who = check_id_token(&tok, "https://auth.openai.com", "oaiapp_1", Some("n")).unwrap();
        assert_eq!(who, Identity { email: "lea@example.com".into(), subject: "user-1".into() });
        assert!(check_id_token(&tok, "https://auth.openai.com", "oaiapp_2", Some("n")).is_err(), "autre destinataire");
        assert!(check_id_token(&tok, "https://auth.openai.com", "oaiapp_1", Some("autre")).is_err(), "autre nonce");
        assert!(check_id_token(&tok, "https://evil.example", "oaiapp_1", Some("n")).is_err(), "autre émetteur");
        let old = jwt(json!({ "iss": "https://auth.openai.com", "aud": ["oaiapp_1"], "exp": 1000, "nonce": "n" }));
        assert!(check_id_token(&old, "https://auth.openai.com", "oaiapp_1", Some("n")).is_err(), "expiré");
        assert!(check_id_token("pas-un-jeton", "https://auth.openai.com", "oaiapp_1", None).is_err());
    }

    #[test]
    fn bodies_move_system_to_instructions() {
        let p = crate::online::find("chatgpt").unwrap();
        let cfg = Config { provider: p, name: "ChatGPT".into(), base: RESOURCE.into(), key: String::new(), model: "gpt-6.1-mini".into() };
        let m = [("system", "Tu traduis.".to_string()), ("user", "ran".to_string()), ("assistant", "Sens : courut".to_string()), ("user", "pesca".to_string())];
        let ask = Ask { messages: &m, max_tokens: 72, think: None, exact: true };
        let b = body(&cfg, &ask, Level::Full);
        assert_eq!((b["store"].as_bool(), b["stream"].as_bool()), (Some(false), Some(true)));
        assert_eq!(b["instructions"], "Tu traduis.");
        assert_eq!(b["input"].as_array().unwrap().len(), 3);
        assert!(!b.to_string().contains("\"system\""), "jamais de message « system » : {b}");
        assert!(b["temperature"].is_null() && b["max_output_tokens"].is_null());
        assert_eq!(b["reasoning"]["effort"], "none");
        assert_eq!(body(&cfg, &ask, Level::Alt)["reasoning"]["effort"], "minimal");
        assert!(body(&cfg, &ask, Level::Plain)["reasoning"].is_null());
        let ask = Ask { messages: &m, max_tokens: 72, think: Some("high"), exact: false };
        assert_eq!(body(&cfg, &ask, Level::Full)["reasoning"], json!({ "effort": "high", "summary": "auto" }));
        // une consigne au milieu de la conversation
        let m = [("user", "a".to_string()), ("system", "Rappel".to_string())];
        let b = body(&cfg, &Ask { messages: &m, max_tokens: 72, think: None, exact: false }, Level::Plain);
        assert_eq!(b["input"][1]["role"], "developer");
        assert!(b["instructions"].is_null());
    }

    #[test]
    fn stream_events_are_read() {
        assert_eq!(parse_event(r#"data: {"type":"response.output_text.delta","delta":"Sens"}"#), Some(Event::Text("Sens".into())));
        assert_eq!(parse_event(r#"data: {"type":"response.reasoning_summary_text.delta","delta":"Hmm"}"#), Some(Event::Thought("Hmm".into())));
        assert_eq!(parse_event(r#"data: {"type":"response.completed","response":{}}"#), Some(Event::Done));
        assert_eq!(parse_event(r#"data: {"type":"response.incomplete"}"#), Some(Event::Done));
        assert_eq!(
            parse_event(r#"data: {"type":"response.failed","response":{"error":{"code":"subscription_sharing_usage_limit_exceeded","message":"limit"}}}"#),
            Some(Event::Failed { code: USAGE_LIMIT.into(), message: "limit".into() })
        );
        assert!(parse_event("event: response.output_text.delta").is_none());
        assert!(parse_event(r#"data: {"type":"response.created"}"#).is_none());
    }

    #[test]
    fn models_are_listed_and_picked() {
        let v = json!({ "models": [
            { "slug": "gpt-6.1-sol", "display_name": "GPT-6.1", "visibility": "list" },
            { "slug": "gpt-6.1-mini", "visibility": "list" },
            { "slug": "gpt-5.4-mini", "visibility": "list" },
            { "slug": "gpt-internal", "visibility": "hide" }
        ]});
        let ids = models_of(&v);
        assert_eq!(ids, ["gpt-6.1-sol", "gpt-6.1-mini", "gpt-5.4-mini"]);
        assert_eq!(pick(&ids).as_deref(), Some("gpt-6.1-mini"));
        assert_eq!(pick(&["gpt-6.1-sol".to_string()]).as_deref(), Some("gpt-6.1-sol"));
        assert_eq!(models_of(&json!({ "data": [{ "id": "gpt-6.1-mini" }] })), ["gpt-6.1-mini"]);
    }

    /// Un faux OpenAI : connexion, jetons, modèles, et réponses au fil de l'eau.
    /// `mode` choisit la réponse de /v1/responses.
    struct Fake {
        base: String,
        posts: Arc<AtomicUsize>,
        mode: Arc<AtomicU64>,
        refreshes: Arc<AtomicUsize>,
        /// paramètres de la dernière page d'autorisation ouverte
        authorize: Arc<Mutex<std::collections::HashMap<String, String>>>,
        /// compte qui répond à la connexion (`sub` du jeton d'identité)
        subject: Arc<Mutex<String>>,
    }

    const OK: u64 = 0;
    const LIMIT: u64 = 1;
    const FAIL_MIDWAY: u64 = 2;
    const EXPIRED: u64 = 3;
    const NOT_ELIGIBLE: u64 = 4;

    async fn fake() -> Fake {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (posts, mode, refreshes) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicU64::new(OK)), Arc::new(AtomicUsize::new(0)));
        let (authorize, subject) = (Arc::new(Mutex::new(std::collections::HashMap::new())), Arc::new(Mutex::new("user-1".to_string())));
        let (p, m, r, au, su, issuer) = (posts.clone(), mode.clone(), refreshes.clone(), authorize.clone(), subject.clone(), base.clone());
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let (p, m, r, au, su, issuer) = (p.clone(), m.clone(), r.clone(), au.clone(), su.clone(), issuer.clone());
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut tmp = [0u8; 8192];
                    let req = loop {
                        let n = sock.read(&mut tmp).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&tmp[..n]);
                        let text = String::from_utf8_lossy(&buf).to_string();
                        if let Some(i) = text.find("\r\n\r\n") {
                            let len = text[..i].lines().find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                            if buf.len() >= i + 4 + len {
                                break text;
                            }
                        }
                    };
                    let reply = |status: &str, kind: &str, body: &str| format!("HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                    let json = "application/json";
                    let pairs = |s: &str| -> std::collections::HashMap<String, String> {
                        reqwest::Url::parse(&format!("http://x/?{s}")).unwrap().query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect()
                    };
                    let out = if req.starts_with("GET /api/accounts/authorize?") {
                        // la page d'autorisation : l'apprenant accepte, retour vers Lumen
                        let target = req.split_whitespace().nth(1).unwrap_or("");
                        let q = pairs(target.split_once('?').map_or("", |x| x.1));
                        let get = |k: &str| q.get(k).cloned().unwrap_or_default();
                        let issued = if get("client_id") == "dynamic_agent_client" { "oaiapp_new".to_string() } else { get("client_id") };
                        let mut to = reqwest::Url::parse(&get("redirect_uri")).unwrap();
                        to.query_pairs_mut().append_pair("code", "c0de").append_pair("state", &get("state")).append_pair("client_id", &issued).append_pair("scope", SCOPES);
                        *au.lock() = q;
                        format!("HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    } else if req.starts_with("POST /api/accounts/oauth/token") {
                        if req.contains("grant_type=refresh_token") {
                            r.fetch_add(1, Ordering::SeqCst);
                            if req.contains("refresh_token=dead") {
                                reply("400 Bad Request", json, r#"{"error":"refresh_token_reused"}"#)
                            } else {
                                reply("200 OK", json, r#"{"access_token":"neuf","refresh_token":"r2","expires_in":3600}"#)
                            }
                        } else {
                            // échange du code : PKCE et adresse de retour vérifiés comme chez OpenAI
                            let f = pairs(req.split_once("\r\n\r\n").map_or("", |x| x.1));
                            let a = au.lock().clone();
                            let get = |m: &std::collections::HashMap<String, String>, k: &str| m.get(k).cloned().unwrap_or_default();
                            let pkce = !get(&a, "code_challenge").is_empty() && get(&a, "code_challenge") == challenge_of(&get(&f, "code_verifier"));
                            if get(&f, "grant_type") != "authorization_code" || get(&f, "code") != "c0de" || !pkce || get(&f, "redirect_uri") != get(&a, "redirect_uri") {
                                reply("400 Bad Request", json, r#"{"error":"invalid_grant"}"#)
                            } else {
                                let claims = json!({ "iss": issuer, "aud": get(&f, "client_id"), "exp": now_secs() + 3600, "nonce": get(&a, "nonce"), "sub": su.lock().clone(), "email": "lea@example.com" });
                                let body = json!({ "id_token": jwt(claims), "access_token": "a1", "refresh_token": "r1", "expires_in": 3600, "scope": SCOPES });
                                reply("200 OK", json, &body.to_string())
                            }
                        }
                    } else if req.starts_with("GET /v1/models") {
                        reply("200 OK", json, r#"{"models":[{"slug":"gpt-6.1-sol","visibility":"list"},{"slug":"gpt-6.1-mini","visibility":"list"}]}"#)
                    } else if req.starts_with("POST /v1/responses") {
                        p.fetch_add(1, Ordering::SeqCst);
                        assert!(req.contains(r#""store":false"#) && req.contains(r#""stream":true"#), "{req}");
                        match m.load(Ordering::SeqCst) {
                            LIMIT => reply("429 Too Many Requests", json, r#"{"error":{"code":"subscription_sharing_usage_limit_exceeded","message":"limit"}}"#),
                            NOT_ELIGIBLE => reply("403 Forbidden", json, r#"{"error":{"code":"subscription_sharing_user_not_eligible","message":"no"}}"#),
                            EXPIRED if !req.contains("Bearer neuf") => reply("401 Unauthorized", json, r#"{"error":{"code":"subscription_sharing_invalid_user","message":"expired"}}"#),
                            FAIL_MIDWAY => reply(
                                "200 OK",
                                "text/event-stream",
                                "data: {\"type\":\"response.created\"}\n\ndata: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"subscription_sharing_usage_limit_exceeded\",\"message\":\"limit\"}}}\n\n",
                            ),
                            _ => {
                                // un peu de latence, comme un vrai serveur
                                tokio::time::sleep(Duration::from_millis(40)).await;
                                reply(
                                    "200 OK",
                                    "text/event-stream",
                                    "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"Sens : pêche\"}\n\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"\\nNote : -\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{}}\n\n",
                                )
                            }
                        }
                    } else {
                        reply("404 Not Found", json, "{}")
                    };
                    let _ = sock.write_all(out.as_bytes()).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        Fake { base, posts, mode, refreshes, authorize, subject }
    }

    fn vault_for(f: &Fake, name: &str, access: &str, refresh: &str) -> Vault {
        let v = Vault::at(temp_path(name), Endpoints { auth: f.base.clone(), api: format!("{}/v1", f.base) });
        v.update(|s| {
            s.account = Some(Account {
                client_id: "oaiapp_test".into(),
                email: "lea@example.com".into(),
                subject: "user-1".into(),
                access_token: access.into(),
                refresh_token: refresh.into(),
                expires_at: now_secs() + 3600,
                scopes: SCOPES.into(),
                ..Default::default()
            })
        });
        v
    }

    fn word<'a>(m: &'a [(&'a str, String)]) -> Ask<'a> {
        Ask { messages: m, max_tokens: 72, think: None, exact: true }
    }

    /// Le souci d'Ulysse : beaucoup de mots touchés en quelques secondes. Chaque
    /// mot rend le précédent caduc (comme l'époque des requêtes dans Lumen) : seuls
    /// les mots où l'on s'arrête partent chez ChatGPT, sans erreur ni blocage.
    #[tokio::test]
    async fn bursts_of_taps_send_only_the_words_you_stop_on() {
        let f = fake().await;
        let v = Arc::new(vault_for(&f, "burst", "a1", "r1"));
        TEST_PACE.with(|x| x.set(Some(Pace { settle: Duration::from_millis(120), per_minute: 30, cooldown: Duration::from_secs(60) })));
        let cfg = config_for(&v, "gpt-6.1-mini");
        let epoch = Arc::new(AtomicU64::new(0));
        let m = [("user", "pesca".to_string())];
        // 20 mots touchés à 50 ms d'intervalle (une rafale), puis on s'arrête sur le dernier
        let mut handles = Vec::new();
        for _ in 0..20 {
            let id = epoch.fetch_add(1, Ordering::SeqCst) + 1;
            let (v, cfg, epoch, m) = (v.clone(), cfg.clone(), epoch.clone(), m.clone());
            handles.push(tokio::spawn(async move {
                let halted = || if epoch.load(Ordering::SeqCst) != id { Err(anyhow!("interrompu")) } else { Ok(false) };
                run_with(&v, &cfg, word(&m), halted, |_| true, true).await
            }));
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let mut answered = 0;
        let mut interrupted = 0;
        for h in handles {
            match h.await.unwrap() {
                Ok(out) if out.answer == "Sens : pêche\nNote : -" => answered += 1,
                Err(e) if e.to_string() == "interrompu" => interrupted += 1,
                other => panic!("ni réponse ni interruption : {:?}", other.map(|o| o.answer)),
            }
        }
        assert_eq!((answered, interrupted), (1, 19), "seul le dernier mot obtient sa traduction");
        assert_eq!(f.posts.load(Ordering::SeqCst), 1, "une seule demande est partie chez ChatGPT");
        assert_eq!(v.status().requests_today, 1);
    }

    /// Mots où l'on s'arrête vraiment, très vite : au-delà de la limite par minute,
    /// les suivants attendent leur tour (et rien ne casse).
    #[tokio::test]
    async fn steady_taps_are_capped_per_minute() {
        let f = fake().await;
        let v = vault_for(&f, "cap", "a1", "r1");
        TEST_PACE.with(|x| x.set(Some(Pace { settle: Duration::ZERO, per_minute: 3, cooldown: Duration::from_secs(60) })));
        let cfg = config_for(&v, "gpt-6.1-mini");
        let m = [("user", "pesca".to_string())];
        for _ in 0..3 {
            assert!(run_with(&v, &cfg, word(&m), || Ok(false), |_| true, true).await.is_ok());
        }
        // la quatrième attendrait une place : l'apprenant passe à un autre mot entre-temps
        let start = Instant::now();
        let halted = move || if start.elapsed() > Duration::from_millis(400) { Err(anyhow!("interrompu")) } else { Ok(false) };
        let e = run_with(&v, &cfg, word(&m), halted, |_| true, true).await.unwrap_err();
        assert_eq!(e.to_string(), "interrompu");
        assert_eq!(f.posts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn plan_limit_pauses_and_falls_back() {
        let f = fake().await;
        let v = vault_for(&f, "limit", "a1", "r1");
        TEST_PACE.with(|x| x.set(Some(Pace { settle: Duration::ZERO, per_minute: 30, cooldown: Duration::from_secs(600) })));
        let cfg = config_for(&v, "gpt-6.1-mini");
        let m = [("user", "pesca".to_string())];
        f.mode.store(LIMIT, Ordering::SeqCst);
        let e = run_with(&v, &cfg, word(&m), || Ok(false), |_| true, true).await.unwrap_err();
        assert!(crate::online::is_unreachable(&e) && e.to_string().contains("limite"), "repli sur l'appareil : {e}");
        // les mots suivants ne redemandent pas pendant la pause
        let e = run_with(&v, &cfg, word(&m), || Ok(false), |_| true, true).await.unwrap_err();
        assert!(crate::online::is_unreachable(&e));
        assert_eq!(f.posts.load(Ordering::SeqCst), 1);
        assert_eq!(v.status().paused_minutes, Some(10));
        // refus en cours de route : même chose
        let v = vault_for(&f, "limit2", "a1", "r1");
        f.mode.store(FAIL_MIDWAY, Ordering::SeqCst);
        let e = run_with(&v, &cfg, word(&m), || Ok(false), |_| true, true).await.unwrap_err();
        assert!(crate::online::is_unreachable(&e), "{e}");
        // compte gratuit : dit clairement, sans repli silencieux
        let v = vault_for(&f, "free", "a1", "r1");
        f.mode.store(NOT_ELIGIBLE, Ordering::SeqCst);
        let e = run_with(&v, &cfg, word(&m), || Ok(false), |_| true, true).await.unwrap_err();
        assert!(!crate::online::is_unreachable(&e) && e.to_string().contains("Plus ou Pro"), "{e}");
    }

    #[tokio::test]
    async fn expired_tokens_are_renewed_once() {
        let f = fake().await;
        TEST_PACE.with(|x| x.set(Some(Pace { settle: Duration::ZERO, per_minute: 30, cooldown: Duration::from_secs(60) })));
        let v = vault_for(&f, "renew", "vieux", "r1");
        let cfg = config_for(&v, "gpt-6.1-mini");
        let m = [("user", "pesca".to_string())];
        f.mode.store(EXPIRED, Ordering::SeqCst);
        let out = run_with(&v, &cfg, word(&m), || Ok(false), |_| true, true).await.unwrap();
        assert_eq!(out.answer, "Sens : pêche\nNote : -");
        assert_eq!(f.refreshes.load(Ordering::SeqCst), 1);
        // le nouveau jeton de renouvellement est gardé (il ne sert qu'une fois)
        let saved: Saved = serde_json::from_slice(&std::fs::read(&v.path).unwrap()).unwrap();
        assert_eq!(saved.account.unwrap().refresh_token, "r2");
        // jeton de renouvellement refusé : connexion à refaire, dite comme telle
        let v = vault_for(&f, "dead", "vieux", "dead");
        let e = run_with(&v, &cfg, word(&m), || Ok(false), |_| true, true).await.unwrap_err();
        assert!(e.to_string().contains("Reconnectez-vous") || e.to_string().contains("Sign in again"), "{e}");
        assert!(v.status().expired && !signed_in_vault(&v));
        // modèles du compte, et la petite vérification
        f.mode.store(OK, Ordering::SeqCst);
        let v = vault_for(&f, "check", "a1", "r1");
        let chk = check_with(&v, "").await.unwrap();
        assert_eq!((chk.models.len(), chk.model.as_str()), (2, "gpt-6.1-mini"));
    }

    fn signed_in_vault(v: &Vault) -> bool {
        let st = v.status();
        st.signed_in && st.plan && !st.expired
    }

    /// Le retour du navigateur : la bonne page, le bon `state`, l'identifiant délivré.
    #[tokio::test]
    async fn browser_callback_is_read() {
        let backs = AtomicUsize::new(0);
        let back = || {
            backs.fetch_add(1, Ordering::SeqCst);
        };
        let listener = bind_loopback().await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let cancel = AtomicBool::new(false);
        let browser = tokio::spawn(async move {
            // le navigateur demande d'abord l'icône, puis revient avec le code
            let _ = client().get(format!("http://127.0.0.1:{port}/favicon.ico")).send().await;
            client().get(format!("http://127.0.0.1:{port}/auth/callback?code=c0de&state=s1&client_id=oaiapp_9&scope=openid")).send().await.unwrap().text().await.unwrap()
        });
        let cb = wait_callback(listener, "s1", &cancel, Duration::from_secs(5), back).await.unwrap();
        assert_eq!(cb, Callback { code: "c0de".into(), client_id: "oaiapp_9".into(), scope: "openid".into() });
        assert!(browser.await.unwrap().contains("Lumen"));
        assert_eq!(backs.load(Ordering::SeqCst), 1, "Lumen repasse devant au retour, pas pour l'icône");
        // refus dans ChatGPT : le navigateur revient aussi
        let listener = bind_loopback().await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let _ = client().get(format!("http://127.0.0.1:{port}/auth/callback?error=access_denied&state=s1")).send().await;
        });
        assert!(wait_callback(listener, "s1", &cancel, Duration::from_secs(5), back).await.is_err());
        assert_eq!(backs.load(Ordering::SeqCst), 2);
        // annulation par l'apprenant : il est déjà dans Lumen
        let listener = bind_loopback().await.unwrap();
        let cancel = AtomicBool::new(true);
        assert_eq!(wait_callback(listener, "s1", &cancel, Duration::from_secs(5), back).await.unwrap_err().to_string(), "annulé");
        assert_eq!(backs.load(Ordering::SeqCst), 2);
    }

    /// La connexion entière, comme sur Mac et Windows : Lumen ouvre le navigateur,
    /// l'apprenant accepte chez (un faux) OpenAI, le navigateur revient sur l'adresse
    /// locale, le code est échangé (PKCE vérifié), le jeton d'identité contrôlé, et
    /// le compte gardé dans chatgpt.json. Puis la reconnexion, et un autre compte.
    #[tokio::test]
    async fn sign_in_goes_all_the_way() {
        let f = fake().await;
        let path = temp_path("signin");
        let v = Vault::at(path.clone(), Endpoints { auth: f.base.clone(), api: format!("{}/v1", f.base) });
        let cancel = AtomicBool::new(false);
        let backs = AtomicUsize::new(0);
        let back = || {
            backs.fetch_add(1, Ordering::SeqCst);
        };
        // le navigateur du système : il suit la page d'autorisation jusqu'au retour
        let browser = || {
            let opened: Arc<Mutex<Option<String>>> = Arc::default();
            let page = opened.clone();
            let open = move |url: &str| -> Result<()> {
                let url = url.to_string();
                tokio::spawn(async move {
                    let text = client().get(url).send().await.unwrap().text().await.unwrap();
                    *page.lock() = Some(text);
                });
                Ok(())
            };
            (open, opened)
        };

        let (open, opened) = browser();
        let st = sign_in_with(&v, open, back, &cancel, Duration::from_secs(10)).await.unwrap();
        assert!(st.signed_in && st.plan && !st.expired, "{st:?}");
        assert_eq!(st.email, "lea@example.com");
        assert_eq!(backs.load(Ordering::SeqCst), 1);
        let first = f.authorize.lock().clone();
        assert_eq!((first["client_id"].as_str(), first["agent_name_hint"].as_str()), ("dynamic_agent_client", "Lumen"));
        assert_eq!((first["resource"].as_str(), first["scope"].as_str()), (RESOURCE, SCOPES));
        assert!(first["redirect_uri"].starts_with("http://127.0.0.1:") && first["redirect_uri"].ends_with(CALLBACK));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(opened.lock().as_deref().is_some_and(|p| p.contains("Lumen")), "page de retour montrée");
        // gardé hors de la base, lisible par l'utilisateur seul, relu au lancement suivant
        let saved: Saved = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let acc = saved.account.unwrap();
        assert_eq!((acc.client_id.as_str(), acc.access_token.as_str(), acc.refresh_token.as_str()), ("oaiapp_new", "a1", "r1"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let again = Vault::at(path.clone(), Endpoints { auth: f.base.clone(), api: format!("{}/v1", f.base) });
        assert!(signed_in_vault(&again));
        assert_eq!(again.saved.lock().host_id, v.saved.lock().host_id, "même appareil pour OpenAI");

        // reconnexion : l'identifiant délivré, le compte désigné, pas de nouvel enregistrement
        let (open, _) = browser();
        assert!(sign_in_with(&v, open, back, &cancel, Duration::from_secs(10)).await.unwrap().signed_in);
        let second = f.authorize.lock().clone();
        assert_eq!((second["client_id"].as_str(), second["login_hint"].as_str()), ("oaiapp_new", "lea@example.com"));
        assert!(second.contains_key("id_token_hint") && !second.contains_key("agent_name_hint"));
        assert_ne!(second["state"], first["state"]);

        // un autre compte répond : refusé, et le compte oublié
        *f.subject.lock() = "user-2".into();
        let (open, _) = browser();
        let e = sign_in_with(&v, open, back, &cancel, Duration::from_secs(10)).await.unwrap_err();
        assert!(e.to_string().contains("autre compte") || e.to_string().contains("different ChatGPT account"), "{e}");
        assert!(!v.status().signed_in);
        assert_eq!(backs.load(Ordering::SeqCst), 3);
    }
}
