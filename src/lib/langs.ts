import type { LangCode } from "./types";

export interface LangInfo {
  code: LangCode;
  name: string; // en français
  native: string;
  badge: string;
  color: string;
  tts: string[]; // préférences de voix (BCP 47)
  /** salutation, pour l'accueil */
  hello: string;
  /** écriture de droite à gauche */
  rtl?: boolean;
}

/**
 * Langues proposées : toutes celles de la voix naturelle (Supertonic 3).
 * Les six premières ont en plus un dictionnaire hors ligne.
 */
export const LANGS: LangInfo[] = [
  { code: "en", name: "Anglais", native: "English", badge: "EN", color: "#3b5bdb", tts: ["en-GB", "en-US", "en"], hello: "Hello" },
  { code: "it", name: "Italien", native: "Italiano", badge: "IT", color: "#2f9e6e", tts: ["it-IT", "it"], hello: "Ciao" },
  { code: "de", name: "Allemand", native: "Deutsch", badge: "DE", color: "#c4512d", tts: ["de-DE", "de"], hello: "Hallo" },
  { code: "pt", name: "Portugais", native: "Português", badge: "PT", color: "#1c8f8a", tts: ["pt-PT", "pt-BR", "pt"], hello: "Olá" },
  { code: "ru", name: "Russe", native: "Русский", badge: "RU", color: "#7048c8", tts: ["ru-RU", "ru"], hello: "Привет" },
  { code: "es", name: "Espagnol", native: "Español", badge: "ES", color: "#d9822b", tts: ["es-ES", "es-MX", "es"], hello: "Hola" },
  { code: "fr", name: "Français", native: "Français", badge: "FR", color: "#5b5fc7", tts: ["fr-FR", "fr"], hello: "Bonjour" },
  { code: "nl", name: "Néerlandais", native: "Nederlands", badge: "NL", color: "#e8590c", tts: ["nl-NL", "nl-BE", "nl"], hello: "Hoi" },
  { code: "sv", name: "Suédois", native: "Svenska", badge: "SV", color: "#1971c2", tts: ["sv-SE", "sv"], hello: "Hej" },
  { code: "da", name: "Danois", native: "Dansk", badge: "DA", color: "#c92a2a", tts: ["da-DK", "da"], hello: "Goddag" },
  { code: "fi", name: "Finnois", native: "Suomi", badge: "FI", color: "#0c8599", tts: ["fi-FI", "fi"], hello: "Hei" },
  { code: "et", name: "Estonien", native: "Eesti", badge: "ET", color: "#4c6ef5", tts: ["et-EE", "et"], hello: "Tere" },
  { code: "lv", name: "Letton", native: "Latviešu", badge: "LV", color: "#a61e4d", tts: ["lv-LV", "lv"], hello: "Sveiki" },
  { code: "lt", name: "Lituanien", native: "Lietuvių", badge: "LT", color: "#a07c00", tts: ["lt-LT", "lt"], hello: "Labas" },
  { code: "pl", name: "Polonais", native: "Polski", badge: "PL", color: "#d6336c", tts: ["pl-PL", "pl"], hello: "Cześć" },
  { code: "cs", name: "Tchèque", native: "Čeština", badge: "CS", color: "#364fc7", tts: ["cs-CZ", "cs"], hello: "Ahoj" },
  { code: "sk", name: "Slovaque", native: "Slovenčina", badge: "SK", color: "#4263eb", tts: ["sk-SK", "sk"], hello: "Dobrý deň" },
  { code: "sl", name: "Slovène", native: "Slovenščina", badge: "SL", color: "#2f9e44", tts: ["sl-SI", "sl"], hello: "Živjo" },
  { code: "hr", name: "Croate", native: "Hrvatski", badge: "HR", color: "#e03131", tts: ["hr-HR", "hr"], hello: "Bok" },
  { code: "hu", name: "Hongrois", native: "Magyar", badge: "HU", color: "#2b8a3e", tts: ["hu-HU", "hu"], hello: "Szia" },
  { code: "ro", name: "Roumain", native: "Română", badge: "RO", color: "#f08c00", tts: ["ro-RO", "ro"], hello: "Salut" },
  { code: "bg", name: "Bulgare", native: "Български", badge: "BG", color: "#0b7285", tts: ["bg-BG", "bg"], hello: "Здравей" },
  { code: "uk", name: "Ukrainien", native: "Українська", badge: "UK", color: "#1864ab", tts: ["uk-UA", "uk"], hello: "Привіт" },
  { code: "el", name: "Grec", native: "Ελληνικά", badge: "EL", color: "#1c7ed6", tts: ["el-GR", "el"], hello: "Γεια σου" },
  { code: "tr", name: "Turc", native: "Türkçe", badge: "TR", color: "#c2255c", tts: ["tr-TR", "tr"], hello: "Merhaba" },
  { code: "ar", name: "Arabe", native: "العربية", badge: "AR", color: "#087f5b", tts: ["ar-SA", "ar-EG", "ar"], hello: "مرحبا", rtl: true },
  { code: "hi", name: "Hindi", native: "हिन्दी", badge: "HI", color: "#e67700", tts: ["hi-IN", "hi"], hello: "नमस्ते" },
  { code: "id", name: "Indonésien", native: "Bahasa Indonesia", badge: "ID", color: "#d9480f", tts: ["id-ID", "id"], hello: "Halo" },
  { code: "vi", name: "Vietnamien", native: "Tiếng Việt", badge: "VI", color: "#c0392b", tts: ["vi-VN", "vi"], hello: "Xin chào" },
  { code: "ko", name: "Coréen", native: "한국어", badge: "KO", color: "#5f3dc4", tts: ["ko-KR", "ko"], hello: "안녕하세요" },
  { code: "ja", name: "Japonais", native: "日本語", badge: "JA", color: "#e64980", tts: ["ja-JP", "ja"], hello: "こんにちは" },
];

/** Les six langues avec un dictionnaire hors ligne (les autres : IA seulement). */
export const CORE_LANGS: LangCode[] = ["en", "it", "de", "pt", "ru", "es"];

export function langInfo(code: string): LangInfo {
  return LANGS.find((l) => l.code === code) ?? LANGS[0];
}

/** « l'italien », « le russe », « le hongrois » (h aspiré). */
export function theLang(code: string): string {
  const n = langInfo(code).name.toLowerCase();
  return /^[aeiouéèêh]/.test(n) && !n.startsWith("hongrois") ? `l'${n}` : `le ${n}`;
}

/** Textes de départ (originaux) pour commencer immédiatement. */
export const STARTERS: Record<LangCode, { title: string; text: string }> = {
  en: {
    title: "The Lighthouse Keeper",
    text: `Every morning, Martha climbed the narrow stairs of the old lighthouse. From the top, the sea looked endless and calm. She liked to listen to the wind while the sun rose slowly behind the clouds.

One day, she found a letter hidden between two stones. The paper was damp, but the words could still be read: "If you are reading this, you are not alone."

Martha sat down and read the letter again. Who had written it? How long had it been there? She looked out at the sea, as if the answer might come with the next wave.

That evening, she wrote a reply and left it in the same place. She did not know if anyone would ever find it. But for the first time in years, the lighthouse did not feel empty.`,
  },
  es: {
    title: "El faro de la isla",
    text: `Cada mañana, Marta subía las escaleras del viejo faro. Desde arriba, el mar parecía infinito y tranquilo. Le gustaba escuchar el viento mientras el sol aparecía lentamente detrás de las nubes.

Un día encontró una carta escondida entre las piedras. El papel estaba húmedo, pero las palabras todavía se podían leer: «Si lees esto, no estás sola».

Marta se sentó y volvió a leer la carta. ¿Quién la había escrito? ¿Cuánto tiempo llevaba allí? Miró el mar, como si la respuesta pudiera llegar con la próxima ola.

Esa noche escribió una respuesta y la dejó en el mismo lugar. No sabía si alguien la encontraría algún día. Pero, por primera vez en muchos años, el faro no le pareció vacío.`,
  },
  it: {
    title: "Il guardiano del faro",
    text: `Ogni mattina Marta saliva le scale strette del vecchio faro. Dall'alto, il mare sembrava infinito e tranquillo. Le piaceva ascoltare il vento mentre il sole sorgeva lentamente dietro le nuvole.

Un giorno trovò una lettera nascosta tra due pietre. La carta era umida, ma le parole si potevano ancora leggere: «Se stai leggendo queste righe, non sei sola».

Marta si sedette e rilesse la lettera. Chi l'aveva scritta? Da quanto tempo era lì? Guardò il mare, come se la risposta potesse arrivare con l'onda successiva.

Quella sera scrisse una risposta e la lasciò nello stesso posto. Non sapeva se qualcuno l'avrebbe mai trovata. Ma per la prima volta dopo tanti anni, il faro non le sembrò vuoto.`,
  },
  de: {
    title: "Die Leuchtturmwärterin",
    text: `Jeden Morgen stieg Marta die schmale Treppe des alten Leuchtturms hinauf. Von oben sah das Meer unendlich und ruhig aus. Sie hörte gern dem Wind zu, während die Sonne langsam hinter den Wolken aufging.

Eines Tages fand sie einen Brief, der zwischen zwei Steinen versteckt war. Das Papier war feucht, aber die Wörter konnte man noch lesen: „Wenn du das liest, bist du nicht allein.“

Marta setzte sich und las den Brief noch einmal. Wer hatte ihn geschrieben? Wie lange lag er schon dort? Sie blickte auf das Meer, als könnte die Antwort mit der nächsten Welle kommen.

Am Abend schrieb sie eine Antwort und legte sie an dieselbe Stelle. Sie wusste nicht, ob jemand sie jemals finden würde. Aber zum ersten Mal seit Jahren fühlte sich der Leuchtturm nicht leer an.`,
  },
  pt: {
    title: "O farol da ilha",
    text: `Todas as manhãs, Marta subia as escadas estreitas do velho farol. Lá de cima, o mar parecia infinito e tranquilo. Gostava de ouvir o vento enquanto o sol nascia devagar por trás das nuvens.

Um dia, encontrou uma carta escondida entre duas pedras. O papel estava húmido, mas as palavras ainda se podiam ler: «Se estás a ler isto, não estás sozinha.»

Marta sentou-se e voltou a ler a carta. Quem a teria escrito? Há quanto tempo estaria ali? Olhou para o mar, como se a resposta pudesse chegar com a próxima onda.

Nessa noite, escreveu uma resposta e deixou-a no mesmo lugar. Não sabia se alguém a encontraria um dia. Mas, pela primeira vez em muitos anos, o farol não lhe pareceu vazio.`,
  },
  ru: {
    title: "Смотрительница маяка",
    text: `Каждое утро Марта поднималась по узкой лестнице старого маяка. Сверху море казалось бесконечным и спокойным. Ей нравилось слушать ветер, пока солнце медленно вставало из-за облаков.

Однажды она нашла письмо, спрятанное между двумя камнями. Бумага была влажной, но слова ещё можно было прочитать: «Если ты читаешь это, ты не одна».

Марта села и перечитала письмо. Кто его написал? Как долго оно там лежало? Она посмотрела на море, как будто ответ мог прийти со следующей волной.

Вечером она написала ответ и оставила его на том же месте. Она не знала, найдёт ли его кто-нибудь. Но впервые за много лет маяк не казался ей пустым.`,
  },
  fr: {
    title: "La gardienne du phare",
    text: `Chaque matin, Marthe montait l'étroit escalier du vieux phare. D'en haut, la mer semblait infinie et calme. Elle aimait écouter le vent pendant que le soleil se levait lentement derrière les nuages.

Un jour, elle trouva une lettre cachée entre deux pierres. Le papier était humide, mais on pouvait encore lire les mots : « Si tu lis ceci, tu n'es pas seule. »

Marthe s'assit et relut la lettre. Qui l'avait écrite ? Depuis combien de temps était-elle là ? Elle regarda la mer, comme si la réponse pouvait arriver avec la prochaine vague.

Ce soir-là, elle écrivit une réponse et la laissa au même endroit. Elle ne savait pas si quelqu'un la trouverait un jour. Mais, pour la première fois depuis des années, le phare ne lui sembla pas vide.`,
  },
  nl: {
    title: "De oude vuurtoren",
    text: `Elke ochtend beklom Marta de smalle trap van de oude vuurtoren. Van bovenaf leek de zee eindeloos en kalm. Ze luisterde graag naar de wind, terwijl de zon langzaam achter de wolken opkwam.

Op een dag vond ze een brief, verstopt tussen twee stenen. Het papier was vochtig, maar de woorden waren nog te lezen: “Als je dit leest, ben je niet alleen.”

Marta ging zitten en las de brief opnieuw. Wie had hem geschreven? Hoe lang lag hij er al? Ze keek uit over de zee, alsof het antwoord met de volgende golf zou kunnen komen.

Die avond schreef ze een antwoord en legde het op dezelfde plek. Ze wist niet of iemand het ooit zou vinden. Maar voor het eerst in jaren voelde de vuurtoren niet leeg.`,
  },
  sv: {
    title: "Fyrvaktaren",
    text: `Varje morgon gick Marta uppför den smala trappan i den gamla fyren. Från toppen såg havet oändligt och lugnt ut. Hon tyckte om att lyssna på vinden medan solen sakta steg bakom molnen.

En dag hittade hon ett brev gömt mellan två stenar. Pappret var fuktigt, men orden gick fortfarande att läsa: ”Om du läser det här är du inte ensam.”

Marta satte sig ner och läste brevet igen. Vem hade skrivit det? Hur länge hade det legat där? Hon tittade ut över havet, som om svaret kunde komma med nästa våg.

Samma kväll skrev hon ett svar och lämnade det på samma ställe. Hon visste inte om någon någonsin skulle hitta det. Men för första gången på många år kändes fyren inte tom.`,
  },
  da: {
    title: "Fyrpasseren",
    text: `Hver morgen gik Marta op ad den smalle trappe i det gamle fyrtårn. Fra toppen så havet uendeligt og roligt ud. Hun kunne lide at lytte til vinden, mens solen langsomt steg op bag skyerne.

En dag fandt hun et brev, der lå gemt mellem to sten. Papiret var fugtigt, men ordene kunne stadig læses: »Hvis du læser dette, er du ikke alene.«

Marta satte sig og læste brevet igen. Hvem havde skrevet det? Hvor længe havde det ligget der? Hun så ud over havet, som om svaret kunne komme med den næste bølge.

Den aften skrev hun et svar og lagde det samme sted. Hun vidste ikke, om nogen nogensinde ville finde det. Men for første gang i mange år føltes fyrtårnet ikke tomt.`,
  },
  fi: {
    title: "Majakanvartija",
    text: `Joka aamu Marta kiipesi vanhan majakan kapeita portaita ylös. Ylhäältä meri näytti loputtomalta ja tyyneltä. Hän kuunteli mielellään tuulta, kun aurinko nousi hitaasti pilvien takaa.

Eräänä päivänä hän löysi kirjeen, joka oli piilotettu kahden kiven väliin. Paperi oli kostea, mutta sanat pystyi yhä lukemaan: ”Jos luet tätä, et ole yksin.”

Marta istuutui ja luki kirjeen uudelleen. Kuka sen oli kirjoittanut? Kuinka kauan se oli ollut siellä? Hän katsoi merelle, ikään kuin vastaus voisi tulla seuraavan aallon mukana.

Sinä iltana hän kirjoitti vastauksen ja jätti sen samaan paikkaan. Hän ei tiennyt, löytäisikö kukaan sitä koskaan. Mutta ensimmäistä kertaa vuosiin majakka ei tuntunut tyhjältä.`,
  },
  et: {
    title: "Tuletornivaht",
    text: `Igal hommikul ronis Marta vana tuletorni kitsast treppi mööda üles. Ülevalt paistis meri lõputu ja rahulik. Talle meeldis kuulata tuult, kui päike tõusis aeglaselt pilvede tagant.

Ühel päeval leidis ta kahe kivi vahele peidetud kirja. Paber oli niiske, kuid sõnu sai veel lugeda: „Kui sa seda loed, ei ole sa üksi.“

Marta istus maha ja luges kirja uuesti. Kes oli selle kirjutanud? Kui kaua oli see seal olnud? Ta vaatas merele, justkui võiks vastus tulla koos järgmise lainega.

Sel õhtul kirjutas ta vastuse ja jättis selle samasse kohta. Ta ei teadnud, kas keegi selle kunagi leiab. Kuid esimest korda aastate jooksul ei tundunud tuletorn tühi.`,
  },
  lv: {
    title: "Bākas sargātāja",
    text: `Katru rītu Marta kāpa pa vecās bākas šaurajām kāpnēm. No augšas jūra izskatījās bezgalīga un mierīga. Viņai patika klausīties vējā, kamēr saule lēnām cēlās aiz mākoņiem.

Kādu dienu viņa atrada vēstuli, kas bija paslēpta starp diviem akmeņiem. Papīrs bija mitrs, taču vārdus vēl varēja izlasīt: „Ja tu to lasi, tu neesi viens.”

Marta apsēdās un vēlreiz izlasīja vēstuli. Kas to bija uzrakstījis? Cik ilgi tā tur bija gulējusi? Viņa raudzījās jūrā, it kā atbilde varētu atnākt ar nākamo vilni.

Tajā vakarā viņa uzrakstīja atbildi un atstāja to tajā pašā vietā. Viņa nezināja, vai kāds to kādreiz atradīs. Taču pirmo reizi daudzu gadu laikā bāka nešķita tukša.`,
  },
  lt: {
    title: "Švyturio prižiūrėtoja",
    text: `Kiekvieną rytą Marta lipdavo siaurais senojo švyturio laiptais. Iš viršaus jūra atrodė begalinė ir rami. Jai patikdavo klausytis vėjo, kol saulė lėtai kildavo už debesų.

Vieną dieną ji rado laišką, paslėptą tarp dviejų akmenų. Popierius buvo drėgnas, bet žodžius dar buvo galima perskaityti: „Jei tai skaitai, tu nesi vienas.“

Marta atsisėdo ir dar kartą perskaitė laišką. Kas jį parašė? Kiek laiko jis ten gulėjo? Ji žvelgė į jūrą, tarsi atsakymas galėtų atkeliauti su kita banga.

Tą vakarą ji parašė atsakymą ir paliko jį toje pačioje vietoje. Ji nežinojo, ar kas nors kada nors jį ras. Tačiau pirmą kartą per daugelį metų švyturys neatrodė tuščias.`,
  },
  pl: {
    title: "Stara latarnia",
    text: `Każdego ranka Marta wspinała się po wąskich schodach starej latarni morskiej. Z góry morze wydawało się bezkresne i spokojne. Lubiła słuchać wiatru, gdy słońce powoli wschodziło zza chmur.

Pewnego dnia znalazła list ukryty między dwoma kamieniami. Papier był wilgotny, ale słowa wciąż dało się przeczytać: „Jeśli to czytasz, nie jesteś sam.”

Marta usiadła i jeszcze raz przeczytała list. Kto go napisał? Jak długo tam leżał? Spojrzała na morze, jakby odpowiedź mogła przyjść z następną falą.

Tego wieczoru napisała odpowiedź i zostawiła ją w tym samym miejscu. Nie wiedziała, czy ktokolwiek ją kiedyś znajdzie. Ale po raz pierwszy od lat latarnia nie wydawała się pusta.`,
  },
  cs: {
    title: "Strážkyně majáku",
    text: `Každé ráno Marta stoupala po úzkých schodech starého majáku. Shora vypadalo moře nekonečné a klidné. Ráda poslouchala vítr, zatímco slunce pomalu vycházelo zpoza mraků.

Jednoho dne našla dopis ukrytý mezi dvěma kameny. Papír byl vlhký, ale slova se ještě dala přečíst: „Jestli tohle čteš, nejsi sám.“

Marta se posadila a přečetla si dopis znovu. Kdo ho napsal? Jak dlouho tam ležel? Dívala se na moře, jako by odpověď mohla přijít s další vlnou.

Ten večer napsala odpověď a nechala ji na stejném místě. Nevěděla, jestli ji někdy někdo najde. Ale poprvé po letech se jí maják nezdál prázdný.`,
  },
  sk: {
    title: "Strážkyňa majáka",
    text: `Každé ráno Marta vystupovala po úzkych schodoch starého majáka. Zhora vyzeralo more nekonečné a pokojné. Rada počúvala vietor, kým slnko pomaly vychádzalo spoza oblakov.

Jedného dňa našla list ukrytý medzi dvoma kameňmi. Papier bol vlhký, ale slová sa ešte dali prečítať: „Ak toto čítaš, nie si sám.“

Marta si sadla a znova si prečítala list. Kto ho napísal? Ako dlho tam ležal? Pozerala sa na more, akoby odpoveď mohla prísť s ďalšou vlnou.

V ten večer napísala odpoveď a nechala ju na tom istom mieste. Nevedela, či ju niekedy niekto nájde. Ale po prvý raz po rokoch sa jej maják nezdal prázdny.`,
  },
  sl: {
    title: "Stari svetilnik",
    text: `Vsako jutro se je Marta povzpela po ozkih stopnicah starega svetilnika. Od zgoraj se je morje zdelo neskončno in mirno. Rada je poslušala veter, medtem ko je sonce počasi vzhajalo izza oblakov.

Nekega dne je našla pismo, skrito med dvema kamnoma. Papir je bil vlažen, a besede je bilo še mogoče prebrati: »Če to bereš, nisi sam.«

Marta je sedla in znova prebrala pismo. Kdo ga je napisal? Kako dolgo je že ležalo tam? Gledala je proti morju, kot da bi odgovor lahko prišel z naslednjim valom.

Tisti večer je napisala odgovor in ga pustila na istem mestu. Ni vedela, ali ga bo kdo kdaj našel. A prvič po dolgih letih se ji svetilnik ni zdel prazen.`,
  },
  hr: {
    title: "Stari svjetionik",
    text: `Marta se svakog jutra penjala uskim stubama starog svjetionika. Odozgo je more izgledalo beskrajno i mirno. Voljela je slušati vjetar dok je sunce polako izlazilo iza oblaka.

Jednog dana pronašla je pismo skriveno između dvaju kamena. Papir je bio vlažan, ali riječi su se još mogle pročitati: „Ako ovo čitaš, nisi sam.”

Marta je sjela i ponovno pročitala pismo. Tko ga je napisao? Koliko je dugo ondje ležalo? Gledala je prema moru, kao da bi odgovor mogao stići sa sljedećim valom.

Te je večeri napisala odgovor i ostavila ga na istom mjestu. Nije znala hoće li ga itko ikada pronaći. Ali prvi put nakon mnogo godina svjetionik joj se nije činio praznim.`,
  },
  hu: {
    title: "A világítótorony őre",
    text: `Márta minden reggel felment a régi világítótorony keskeny lépcsőjén. Fentről a tenger végtelennek és nyugodtnak látszott. Szerette hallgatni a szelet, miközben a nap lassan felkelt a felhők mögül.

Egy nap levelet talált két kő közé rejtve. A papír nyirkos volt, de a szavakat még el lehetett olvasni: „Ha ezt olvasod, nem vagy egyedül.”

Márta leült, és újra elolvasta a levelet. Ki írhatta? Mióta lehetett ott? A tengert nézte, mintha a válasz a következő hullámmal érkezhetne.

Aznap este választ írt, és ugyanott hagyta. Nem tudta, megtalálja-e valaha valaki. De évek óta először a világítótorony nem tűnt üresnek.`,
  },
  ro: {
    title: "Paznica farului",
    text: `În fiecare dimineață, Marta urca scările înguste ale vechiului far. De sus, marea părea nesfârșită și liniștită. Îi plăcea să asculte vântul în timp ce soarele răsărea încet din spatele norilor.

Într-o zi, a găsit o scrisoare ascunsă între două pietre. Hârtia era umedă, dar cuvintele se mai puteau citi: „Dacă citești asta, nu ești singur.”

Marta s-a așezat și a recitit scrisoarea. Cine o scrisese? De cât timp era acolo? S-a uitat spre mare, ca și cum răspunsul ar fi putut veni odată cu următorul val.

În seara aceea, a scris un răspuns și l-a lăsat în același loc. Nu știa dacă cineva îl va găsi vreodată. Dar pentru prima dată după mulți ani, farul nu i s-a mai părut gol.`,
  },
  bg: {
    title: "Пазачката на фара",
    text: `Всяка сутрин Марта изкачваше тясното стълбище на стария фар. Отгоре морето изглеждаше безкрайно и спокойно. Обичаше да слуша вятъра, докато слънцето бавно изгряваше зад облаците.

Един ден тя намери писмо, скрито между два камъка. Хартията беше влажна, но думите още можеха да се прочетат: „Ако четеш това, не си сам.“

Марта седна и прочете писмото отново. Кой го беше написал? Откога беше там? Тя гледаше към морето, сякаш отговорът можеше да дойде със следващата вълна.

Същата вечер тя написа отговор и го остави на същото място. Не знаеше дали някой някога ще го намери. Но за първи път от години фарът не ѝ се стори празен.`,
  },
  uk: {
    title: "Доглядачка маяка",
    text: `Щоранку Марта підіймалася вузькими сходами старого маяка. Згори море здавалося безкраїм і спокійним. Їй подобалося слухати вітер, поки сонце повільно сходило з-за хмар.

Одного дня вона знайшла лист, захований між двома каменями. Папір був вологий, але слова ще можна було прочитати: «Якщо ти це читаєш, ти не самотній».

Марта сіла й знову перечитала лист. Хто його написав? Як довго він там лежав? Вона дивилася на море, ніби відповідь могла прийти з наступною хвилею.

Того вечора вона написала відповідь і залишила її на тому самому місці. Вона не знала, чи хтось колись її знайде. Але вперше за багато років маяк не здавався їй порожнім.`,
  },
  el: {
    title: "Η φύλακας του φάρου",
    text: `Κάθε πρωί η Μάρθα ανέβαινε τη στενή σκάλα του παλιού φάρου. Από ψηλά η θάλασσα έμοιαζε ατελείωτη και ήρεμη. Της άρεσε να ακούει τον άνεμο, ενώ ο ήλιος ανέτελλε αργά πίσω από τα σύννεφα.

Μια μέρα βρήκε ένα γράμμα κρυμμένο ανάμεσα σε δύο πέτρες. Το χαρτί ήταν υγρό, αλλά οι λέξεις διαβάζονταν ακόμα: «Αν το διαβάζεις αυτό, δεν είσαι μόνος».

Η Μάρθα κάθισε και ξαναδιάβασε το γράμμα. Ποιος το είχε γράψει; Πόσο καιρό βρισκόταν εκεί; Κοίταξε τη θάλασσα, σαν να μπορούσε η απάντηση να έρθει με το επόμενο κύμα.

Εκείνο το βράδυ έγραψε μια απάντηση και την άφησε στο ίδιο σημείο. Δεν ήξερε αν θα τη βρει ποτέ κανείς. Όμως για πρώτη φορά εδώ και χρόνια ο φάρος δεν της φάνηκε άδειος.`,
  },
  tr: {
    title: "Deniz feneri bekçisi",
    text: `Marta her sabah eski deniz fenerinin dar merdivenlerini tırmanırdı. Yukarıdan deniz uçsuz bucaksız ve sakin görünürdü. Güneş bulutların ardından yavaşça doğarken rüzgârı dinlemeyi severdi.

Bir gün iki taşın arasına saklanmış bir mektup buldu. Kâğıt nemliydi ama kelimeler hâlâ okunabiliyordu: “Bunu okuyorsan, yalnız değilsin.”

Marta oturdu ve mektubu yeniden okudu. Onu kim yazmıştı? Ne zamandır oradaydı? Sanki cevap bir sonraki dalgayla gelebilirmiş gibi denize baktı.

O akşam bir cevap yazdı ve onu aynı yere bıraktı. Birinin onu bir gün bulup bulmayacağını bilmiyordu. Ama yıllardır ilk kez deniz feneri ona boş gelmedi.`,
  },
  ar: {
    title: "حارسة المنارة",
    text: `كل صباح، كانت مارتا تصعد الدرج الضيق للمنارة القديمة. من الأعلى، كان البحر يبدو هادئًا بلا نهاية. كانت تحب أن تصغي إلى الريح بينما تشرق الشمس ببطء من خلف الغيوم.

ذات يوم، وجدت رسالة مخبأة بين حجرين. كانت الورقة رطبة، لكن الكلمات كانت لا تزال مقروءة: «إذا كنت تقرأ هذا، فأنت لست وحدك».

جلست مارتا وقرأت الرسالة من جديد. من كتبها؟ منذ متى وهي هناك؟ نظرت إلى البحر، كأن الجواب قد يأتي مع الموجة التالية.

في ذلك المساء، كتبت ردًا وتركته في المكان نفسه. لم تكن تعرف إن كان أحد سيجده يومًا. لكن للمرة الأولى منذ سنوات، لم تبدُ المنارة فارغة.`,
  },
  hi: {
    title: "पुराना प्रकाशस्तंभ",
    text: `हर सुबह मार्था पुराने प्रकाशस्तंभ की संकरी सीढ़ियाँ चढ़ती थी। ऊपर से समुद्र अनंत और शांत दिखाई देता था। उसे हवा की आवाज़ सुनना अच्छा लगता था, जब सूरज धीरे-धीरे बादलों के पीछे से उगता था।

एक दिन उसे दो पत्थरों के बीच छिपा हुआ एक पत्र मिला। काग़ज़ नम था, लेकिन शब्द अब भी पढ़े जा सकते थे: "अगर तुम यह पढ़ रहे हो, तो तुम अकेले नहीं हो।"

मार्था बैठ गई और उसने पत्र फिर से पढ़ा। इसे किसने लिखा था? यह कब से वहाँ था? उसने समुद्र की ओर देखा, मानो जवाब अगली लहर के साथ आ सकता हो।

उस शाम उसने एक जवाब लिखा और उसे उसी जगह छोड़ दिया। उसे नहीं पता था कि कोई कभी उसे ढूँढ़ेगा या नहीं। लेकिन बरसों में पहली बार प्रकाशस्तंभ उसे खाली नहीं लगा।`,
  },
  id: {
    title: "Penjaga mercusuar",
    text: `Setiap pagi, Marta menaiki tangga sempit di mercusuar tua itu. Dari atas, laut tampak tak berujung dan tenang. Ia suka mendengarkan angin sementara matahari terbit perlahan di balik awan.

Suatu hari, ia menemukan sepucuk surat yang tersembunyi di antara dua batu. Kertasnya lembap, tetapi kata-katanya masih bisa dibaca: "Jika kamu membaca ini, kamu tidak sendirian."

Marta duduk dan membaca surat itu sekali lagi. Siapa yang menulisnya? Sudah berapa lama surat itu ada di sana? Ia memandang ke laut, seolah-olah jawabannya bisa datang bersama ombak berikutnya.

Malam itu, ia menulis balasan dan meninggalkannya di tempat yang sama. Ia tidak tahu apakah suatu hari ada orang yang akan menemukannya. Namun, untuk pertama kalinya dalam bertahun-tahun, mercusuar itu tidak terasa kosong.`,
  },
  vi: {
    title: "Người gác hải đăng",
    text: `Mỗi sáng, Marta leo lên những bậc thang hẹp của ngọn hải đăng cũ. Từ trên cao, biển trông bao la và yên bình. Cô thích lắng nghe tiếng gió trong khi mặt trời từ từ nhô lên sau những đám mây.

Một ngày nọ, cô tìm thấy một lá thư được giấu giữa hai hòn đá. Tờ giấy bị ẩm, nhưng những dòng chữ vẫn còn đọc được: "Nếu bạn đang đọc những dòng này, bạn không hề cô đơn."

Marta ngồi xuống và đọc lại lá thư. Ai đã viết nó? Nó đã nằm ở đó bao lâu rồi? Cô nhìn ra biển, như thể câu trả lời có thể đến cùng con sóng tiếp theo.

Tối hôm đó, cô viết một lá thư hồi âm và để lại ở đúng chỗ cũ. Cô không biết liệu có ai sẽ tìm thấy nó hay không. Nhưng lần đầu tiên sau nhiều năm, ngọn hải đăng không còn trống trải nữa.`,
  },
  ko: {
    title: "등대지기",
    text: `매일 아침 마르타는 오래된 등대의 좁은 계단을 올라갔다. 꼭대기에서 바라본 바다는 끝없이 넓고 고요했다. 그녀는 해가 구름 뒤에서 천천히 떠오르는 동안 바람 소리를 듣는 것을 좋아했다.

어느 날, 그녀는 두 돌 사이에 숨겨진 편지 한 통을 발견했다. 종이는 축축했지만 글씨는 아직 읽을 수 있었다. "이 글을 읽고 있다면, 당신은 혼자가 아니에요."

마르타는 자리에 앉아 편지를 다시 읽었다. 누가 이 편지를 썼을까? 얼마나 오랫동안 여기에 있었을까? 그녀는 마치 다음 파도와 함께 대답이 올지도 모른다는 듯이 바다를 바라보았다.

그날 저녁, 그녀는 답장을 써서 같은 자리에 두었다. 누군가 언젠가 그것을 발견할지는 알 수 없었다. 하지만 몇 년 만에 처음으로 등대가 텅 비어 있다고 느껴지지 않았다.`,
  },
  ja: {
    title: "灯台守",
    text: `毎朝、マルタは古い灯台の狭い階段を上った。上から見る海は果てしなく、穏やかだった。太陽が雲の向こうからゆっくり昇るあいだ、彼女は風の音に耳を傾けるのが好きだった。

ある日、彼女は二つの石のあいだに隠された手紙を見つけた。紙は湿っていたが、文字はまだ読むことができた。「これを読んでいるなら、あなたはひとりじゃない。」

マルタは腰を下ろし、もう一度手紙を読んだ。誰が書いたのだろう。どれくらいのあいだ、ここにあったのだろう。まるで次の波と一緒に答えがやって来るかのように、彼女は海を見つめた。

その晩、彼女は返事を書いて、同じ場所に置いた。いつか誰かがそれを見つけるかどうかはわからなかった。それでも何年ぶりかで、灯台はがらんとしているようには感じなかった。`,
  },
};

export const LEVELS = [
  { id: "A1", label: "A1", hint: "Débutant" },
  { id: "A2", label: "A2", hint: "Élémentaire" },
  { id: "B1", label: "B1", hint: "Intermédiaire" },
  { id: "B2", label: "B2", hint: "Avancé" },
];

/** Paliers de vocabulaire (mots connus), inspirés des repères usuels. */
export const MILESTONES = [
  { words: 500, label: "Premiers pas" },
  { words: 1500, label: "Survie" },
  { words: 3000, label: "Conversation simple" },
  { words: 6000, label: "Lecture courante" },
  { words: 10000, label: "Aisance" },
  { words: 16000, label: "Lecture littéraire" },
  { words: 25000, label: "Maîtrise" },
  { words: 40000, label: "Quasi natif" },
];
