//! Reconnaissance grossière de la langue d'un texte (titre et description d'une
//! vidéo, d'un épisode, paroles d'une chanson), pour ranger les résultats de
//! recherche : ceux qui sont dans la langue étudiée d'abord. Écriture d'abord
//! (cyrillique, grec, arabe, devanagari, hangul, kana), puis mots très fréquents
//! et lettres propres à chaque langue. Sans certitude, rien n'est affirmé.

/// Mots outils très fréquents, propres autant que possible à chaque langue.
const WORDS: &[(&str, &[&str])] = &[
    ("en", &["the", "and", "of", "to", "is", "that", "it", "you", "was", "for", "on", "are", "with", "this", "be", "have", "not", "but", "they", "what", "we", "my", "your", "from", "how", "why", "when", "i'm", "don't", "about"]),
    ("es", &["el", "los", "las", "que", "y", "en", "una", "es", "por", "con", "para", "lo", "como", "más", "pero", "su", "del", "muy", "está", "yo", "qué", "cómo", "también", "porque", "hay", "sus", "fue", "este", "esta", "tu", "hacer", "todo", "todos", "cuando", "donde", "ahora", "bien", "mucho", "nos", "les"]),
    ("fr", &["le", "les", "des", "et", "est", "une", "que", "qui", "dans", "pour", "pas", "sur", "ce", "il", "je", "vous", "avec", "au", "du", "mais", "nous", "c'est", "très", "j'ai", "comment", "pourquoi", "être", "avoir", "son", "ses", "faire", "tout", "tous", "quand", "où", "aussi", "bien", "leur", "cette", "aux"]),
    ("de", &["der", "die", "das", "und", "ist", "nicht", "ein", "eine", "ich", "sie", "es", "zu", "mit", "den", "von", "auf", "auch", "sich", "dem", "wir", "aber", "wie", "für", "noch", "war", "warum", "wird", "sind", "nach", "einen"]),
    ("it", &["il", "lo", "gli", "che", "di", "è", "una", "per", "non", "sono", "con", "mi", "ti", "del", "della", "ma", "anche", "questo", "cosa", "come", "perché", "nel", "alla", "delle", "dei", "più", "io", "ci", "ho", "molto", "tutti", "tutto", "questa", "sei", "loro", "ancora", "quando", "bene", "fare", "nella", "sul", "degli"]),
    ("pt", &["os", "que", "de", "é", "um", "uma", "não", "com", "para", "do", "da", "mas", "eu", "você", "na", "no", "isso", "muito", "está", "ele", "ela", "como", "mais", "dos", "das", "pelo", "pela", "também", "são", "foi"]),
    ("nl", &["de", "het", "een", "en", "van", "is", "dat", "niet", "ik", "je", "op", "te", "zijn", "met", "voor", "er", "maar", "ook", "wat", "als", "aan", "hij", "we", "dit", "naar", "wij", "hoe", "waarom", "nog", "jij"]),
    ("sv", &["och", "att", "det", "är", "som", "en", "på", "för", "med", "jag", "inte", "har", "av", "till", "den", "om", "så", "ett", "var", "men", "vi", "du", "kan", "vad", "hur", "varför", "också", "eller", "sig", "från"]),
    ("da", &["og", "at", "det", "er", "en", "til", "på", "af", "med", "jeg", "ikke", "har", "den", "for", "som", "et", "så", "var", "men", "vi", "du", "kan", "hvad", "også", "hvordan", "hvorfor", "eller", "fra", "nu", "skal"]),
    ("fi", &["ja", "on", "ei", "se", "että", "hän", "oli", "mutta", "kun", "niin", "minä", "sinä", "tämä", "joka", "ovat", "myös", "vain", "jos", "mitä", "olen", "nyt", "kuin", "siitä", "ole", "miten", "miksi", "kanssa", "tai", "vielä", "tässä"]),
    ("et", &["ja", "on", "ei", "et", "see", "ta", "oli", "mis", "kui", "ma", "aga", "siis", "nii", "mida", "seda", "ka", "või", "kes", "olen", "oma", "sa", "kõik", "mina", "veel", "tema", "miks", "kuidas", "selle", "ning", "üle"]),
    ("lv", &["un", "ir", "ka", "ar", "no", "uz", "par", "kas", "es", "tu", "viņš", "bet", "arī", "nav", "tas", "ko", "lai", "man", "mēs", "šis", "bija", "vai", "kā", "kāpēc", "jūs", "viņa", "tikai", "vēl", "kur", "tā"]),
    ("lt", &["ir", "kad", "yra", "su", "į", "iš", "ne", "tai", "bet", "aš", "tu", "jis", "man", "kaip", "mes", "buvo", "taip", "ar", "apie", "nuo", "kur", "dar", "labai", "kas", "jo", "kodėl", "jūs", "jie", "tik", "savo"]),
    ("pl", &["i", "w", "nie", "się", "na", "to", "że", "jest", "z", "do", "jak", "co", "ale", "tak", "ja", "ty", "o", "mnie", "już", "po", "za", "tylko", "czy", "jego", "być", "dlaczego", "jestem", "może", "bardzo", "przez"]),
    ("cs", &["a", "je", "se", "na", "to", "že", "v", "s", "z", "do", "jsem", "ale", "jak", "tak", "co", "by", "už", "jsou", "pro", "od", "jen", "také", "tam", "mi", "být", "proč", "není", "když", "jeho", "které"]),
    ("sk", &["a", "je", "sa", "na", "to", "že", "v", "s", "z", "do", "som", "ale", "ako", "tak", "čo", "by", "už", "sú", "pre", "od", "len", "aj", "tam", "mi", "byť", "prečo", "nie", "keď", "jeho", "ktoré"]),
    ("sl", &["in", "je", "se", "na", "da", "so", "v", "z", "ne", "to", "za", "pa", "kot", "tudi", "sem", "ki", "bi", "ali", "po", "od", "smo", "jaz", "ti", "že", "kaj", "zakaj", "kako", "lahko", "zelo", "bo"]),
    ("hr", &["i", "je", "se", "u", "na", "da", "su", "za", "ne", "to", "od", "a", "sam", "kao", "ali", "što", "ja", "ti", "bi", "će", "iz", "smo", "li", "sve", "koji", "zašto", "kako", "jer", "biti", "može"]),
    ("hu", &["a", "az", "és", "hogy", "nem", "is", "egy", "van", "meg", "de", "ez", "csak", "már", "mint", "volt", "el", "még", "ki", "mi", "én", "te", "azt", "nagyon", "ha", "vagy", "miért", "hogyan", "lesz", "kell", "nincs"]),
    ("ro", &["și", "de", "la", "în", "nu", "este", "pe", "cu", "o", "un", "că", "se", "ce", "mai", "din", "pentru", "sunt", "eu", "tu", "el", "ea", "dar", "cum", "foarte", "care", "fost", "sau", "acest", "această", "și"]),
    ("tr", &["ve", "bir", "bu", "da", "de", "için", "ne", "ben", "sen", "o", "çok", "ama", "gibi", "var", "yok", "daha", "mi", "ile", "değil", "kadar", "şey", "olarak", "her", "biz", "onu", "neden", "nasıl", "olan", "sonra", "şimdi"]),
    ("id", &["yang", "dan", "di", "ini", "itu", "dengan", "untuk", "tidak", "dari", "ada", "saya", "kamu", "akan", "juga", "ke", "ya", "apa", "bisa", "kita", "mereka", "sudah", "karena", "atau", "dalam", "aku", "bagaimana", "kenapa", "lebih", "orang", "jadi"]),
    ("vi", &["và", "của", "là", "có", "không", "được", "một", "những", "trong", "cho", "này", "với", "người", "các", "đã", "tôi", "bạn", "khi", "thì", "cũng", "như", "để", "về", "rất", "nhưng"]),
    ("ru", &["и", "в", "не", "на", "я", "что", "он", "с", "это", "как", "а", "по", "но", "они", "ты", "мы", "вы", "все", "так", "его", "за", "было", "ещё", "уже", "если", "почему", "только", "когда", "очень", "был", "который", "которые", "всегда", "можно", "нужно", "сейчас", "тоже", "чтобы", "быть", "этот", "эта", "кто", "где", "свой", "или", "меня"]),
    ("uk", &["і", "в", "не", "на", "що", "я", "з", "це", "як", "а", "та", "але", "він", "ти", "ми", "ви", "є", "так", "був", "для", "від", "її", "їх", "також", "чому", "дуже", "коли", "тільки", "щоб", "буде", "який", "які", "завжди", "можна", "треба", "зараз", "теж", "бути", "цей", "ця", "хто", "де", "свій", "або", "мене"]),
    ("bg", &["и", "в", "не", "на", "да", "се", "че", "е", "за", "от", "с", "това", "как", "но", "той", "ти", "ние", "вие", "са", "ще", "беше", "със", "който", "тя", "защо", "много", "когато", "само", "съм", "има", "която", "които", "винаги", "трябва", "сега", "също", "този", "тази", "кой", "къде", "или", "мен", "ли", "бъде"]),
];

/// Lettres propres à quelques langues (un indice, pas une preuve).
const LETTERS: &[(&str, &str)] = &[
    ("es", "ñ¿¡"),
    ("fr", "çœêëîïûùàâ"),
    ("de", "ßäöü"),
    ("pt", "ãõçâêô"),
    ("it", "àèìòù"),
    ("sv", "åäö"),
    ("da", "æøå"),
    ("fi", "äö"),
    ("et", "õäöüšž"),
    ("lv", "āēīūķļņģčšž"),
    ("lt", "ąčęėįšųūž"),
    ("pl", "ąęłśźżćń"),
    ("cs", "řůěčšžýáíé"),
    ("sk", "ľĺŕôäčšž"),
    ("sl", "čšž"),
    ("hr", "čćđšž"),
    ("hu", "őűáéíóöúü"),
    ("ro", "șțăîâşţ"),
    ("tr", "ğşıİçöü"),
    ("ru", "ыэё"),
    ("uk", "іїєґ"),
    ("bg", "ъщ"),
];

/// Lettres du vietnamien (voyelles à crochet, đ, tons sous la lettre).
fn vietnamese(c: char) -> bool {
    "ơưđạảấầẩẫậắằẳẵặẹẻẽếềểễệỉịọỏốồổỗộớờởỡợụủứừửữựỳỵỷỹ".contains(c)
}

/// Langue probable d'un texte, ou rien si le texte est trop court ou ambigu.
pub fn guess(text: &str) -> Option<&'static str> {
    let low = text.to_lowercase();
    let (mut latin, mut cyr, mut greek, mut arabic, mut deva, mut hangul, mut kana, mut han, mut viet) = (0usize, 0, 0, 0, 0, 0, 0, 0, 0);
    for c in low.chars() {
        match c {
            '\u{400}'..='\u{4ff}' => cyr += 1,
            '\u{370}'..='\u{3ff}' | '\u{1f00}'..='\u{1fff}' => greek += 1,
            '\u{600}'..='\u{6ff}' | '\u{750}'..='\u{77f}' => arabic += 1,
            '\u{900}'..='\u{97f}' => deva += 1,
            '\u{ac00}'..='\u{d7af}' | '\u{1100}'..='\u{11ff}' => hangul += 1,
            '\u{3040}'..='\u{30ff}' => kana += 1,
            '\u{4e00}'..='\u{9fff}' => han += 1,
            // latin, avec ses lettres accentuées (vietnamien compris)
            _ if c.is_alphabetic() && (c < '\u{370}' || ('\u{1e00}'..='\u{1eff}').contains(&c)) => latin += 1,
            _ => {}
        }
        if vietnamese(c) {
            viet += 1;
        }
    }
    let total = latin + cyr + greek + arabic + deva + hangul + kana + han;
    if total < 8 {
        return None;
    }
    let share = |n: usize| n as f64 / total as f64;
    // écritures propres à une seule langue de Lumen
    if share(kana) > 0.1 || (share(han) > 0.3 && kana > 0) {
        return Some("ja");
    }
    if share(hangul) > 0.2 {
        return Some("ko");
    }
    if share(greek) > 0.4 {
        return Some("el");
    }
    if share(arabic) > 0.4 {
        return Some("ar");
    }
    if share(deva) > 0.4 {
        return Some("hi");
    }
    if share(han) > 0.3 {
        // chinois : aucune langue de Lumen
        return None;
    }
    let cyrillic = share(cyr) > 0.4;
    if !cyrillic && share(latin) < 0.4 {
        return None;
    }
    if !cyrillic && viet * 25 >= latin.max(1) {
        return Some("vi");
    }
    let tokens: Vec<&str> = low
        .split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '’'))
        .map(|w| w.trim_matches(|c| c == '\'' || c == '’'))
        .filter(|w| !w.is_empty())
        .collect();
    if tokens.len() < 4 {
        return None;
    }
    let cyr_langs = ["ru", "uk", "bg"];
    let mut scores: Vec<(&'static str, f64)> = WORDS
        .iter()
        .filter(|(l, _)| cyr_langs.contains(l) == cyrillic)
        .map(|(l, words)| {
            let hits = tokens.iter().filter(|t| words.contains(t)).count() as f64;
            let letters = LETTERS.iter().find(|(x, _)| x == l).map_or(0.0, |(_, set)| {
                let n = low.chars().filter(|c| set.contains(*c)).count() as f64;
                (n * 0.4).min(tokens.len() as f64 * 0.3)
            });
            (*l, hits + letters)
        })
        .collect();
    scores.sort_by(|a, b| b.1.total_cmp(&a.1));
    let (best, s1) = scores[0];
    let s2 = scores.get(1).map_or(0.0, |x| x.1);
    // assez d'indices, et nettement plus que pour la langue suivante
    let enough = s1 >= 2.0 && s1 >= tokens.len() as f64 * 0.08;
    (enough && s1 - s2 >= 1.0 && s1 >= s2 * 1.2).then_some(best)
}

/// Le texte est-il dans cette langue ? `None` : impossible à dire.
pub fn matches(text: &str, lang: &str) -> Option<bool> {
    let g = guess(text)?;
    // tchèque et slovaque se ressemblent trop pour trancher sur un titre
    let close = |a: &str, b: &str| matches!((a, b), ("cs", "sk") | ("sk", "cs") | ("hr", "sl") | ("sl", "hr"));
    Some(g == lang || close(g, lang))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guesses() {
        assert_eq!(guess("Come fare la pasta fresca in casa: la ricetta della nonna, con tutti i segreti"), Some("it"));
        assert_eq!(guess("Wie man frischen Kaffee kocht und warum das nicht so einfach ist"), Some("de"));
        assert_eq!(guess("Comment faire une pâte à pizza maison avec les conseils d'un chef"), Some("fr"));
        assert_eq!(guess("Cómo hacer una paella para toda la familia, paso a paso y muy fácil"), Some("es"));
        assert_eq!(guess("How to make fresh pasta at home with the tips of a real chef"), Some("en"));
        assert_eq!(guess("Как приготовить борщ дома: рецепт, который всегда получается"), Some("ru"));
        assert_eq!(guess("Як приготувати борщ удома: рецепт, який завжди виходить і дуже смачний"), Some("uk"));
        assert_eq!(guess("Cách nấu phở bò ngon tại nhà cho cả gia đình"), Some("vi"));
        assert_eq!(guess("ラーメンの作り方を説明します"), Some("ja"));
        assert_eq!(guess("집에서 김치찌개 만드는 방법을 알려드립니다"), Some("ko"));
        assert_eq!(guess("Πώς να φτιάξετε μουσακά στο σπίτι"), Some("el"));
        // trop court pour trancher
        assert_eq!(guess("Pasta"), None);
        assert_eq!(matches("Come fare la pasta fresca in casa con la nonna", "it"), Some(true));
        assert_eq!(matches("How to make fresh pasta at home with the tips of a chef", "it"), Some(false));
    }
}
