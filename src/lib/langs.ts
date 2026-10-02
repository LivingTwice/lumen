import type { LangCode } from "./types";

export interface LangInfo {
  code: LangCode;
  name: string; // en français
  native: string;
  badge: string;
  color: string;
  tts: string[]; // préférences de voix (BCP 47)
}

export const LANGS: LangInfo[] = [
  { code: "en", name: "Anglais", native: "English", badge: "EN", color: "#3b5bdb", tts: ["en-GB", "en-US", "en"] },
  { code: "it", name: "Italien", native: "Italiano", badge: "IT", color: "#2f9e6e", tts: ["it-IT", "it"] },
  { code: "de", name: "Allemand", native: "Deutsch", badge: "DE", color: "#c4512d", tts: ["de-DE", "de"] },
  { code: "pt", name: "Portugais", native: "Português", badge: "PT", color: "#1c8f8a", tts: ["pt-PT", "pt-BR", "pt"] },
  { code: "ru", name: "Russe", native: "Русский", badge: "RU", color: "#7048c8", tts: ["ru-RU", "ru"] },
  { code: "es", name: "Espagnol", native: "Español", badge: "ES", color: "#d9822b", tts: ["es-ES", "es-MX", "es"] },
];

export function langInfo(code: string): LangInfo {
  return LANGS.find((l) => l.code === code) ?? LANGS[0];
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
