//! Every sentence Sardi says, one table per language.
//!
//! The twin of [`crate::i18n`], and split from it along a line worth keeping:
//! that module holds labels, this one holds sentences whose wording changes with
//! a number. A label is a string; "kept 1 memory" against "kept 3 memories" is a
//! rule about counting, and the rule is not the same in every language.

use crate::settings::Interface;

pub struct Counted {
    pub one: &'static str,
    pub few: &'static str,
    pub many: &'static str,
}

impl Counted {
    const fn same(one: &'static str, many: &'static str) -> Self {
        Self {
            one,
            few: many,
            many,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plural {
    One,
    Few,
    Many,
}

/// The form a count takes in a language.
///
/// Only Polish distinguishes `Few`, and the rule is the ordinary one: the last
/// digit is 2, 3 or 4, except in the teens, where 12–14 go with `Many`. Getting
/// this wrong is not a rounding error in a language — `2 wspomnień` reads the
/// way `2 memory` does.
pub fn plural(language: Interface, count: i64) -> Plural {
    if count == 1 {
        return Plural::One;
    }
    if language != Interface::Polish {
        return Plural::Many;
    }
    let count = count.unsigned_abs();
    match (count % 10, count % 100) {
        (2..=4, 12..=14) => Plural::Many,
        (2..=4, _) => Plural::Few,
        _ => Plural::Many,
    }
}

impl Counted {
    pub fn pick(&self, language: Interface, count: i64) -> &'static str {
        match plural(language, count) {
            Plural::One => self.one,
            Plural::Few => self.few,
            Plural::Many => self.many,
        }
    }
}

pub struct Lines {
    pub reading: &'static str,
    pub adopted_none: &'static str,
    pub adopted: Counted,
    pub listening: &'static str,
    pub available: &'static str,
    pub idle: &'static str,
    pub watching: &'static str,
    pub empty: &'static str,
    pub remembers: Counted,
    pub due: Counted,
    pub restored: Counted,
    pub recalls: Counted,
    pub captured: Counted,
    pub nudge: &'static str,
    pub minutes: Counted,
    pub hours: Counted,
    pub days: Counted,
}

#[cfg(test)]
impl Lines {
    pub fn sentences(&self) -> Vec<&'static str> {
        let Self {
            reading,
            adopted_none,
            adopted,
            listening,
            available,
            idle,
            watching,
            empty,
            remembers,
            due,
            restored,
            recalls,
            captured,
            nudge,
            minutes: _,
            hours: _,
            days: _,
        } = self;
        let mut all = vec![
            *reading,
            *adopted_none,
            *listening,
            *available,
            *idle,
            *watching,
            *empty,
            *nudge,
        ];
        for counted in [adopted, remembers, due, restored, recalls, captured] {
            all.extend([counted.one, counted.few, counted.many]);
        }
        all
    }

    pub fn spans(&self) -> Vec<&'static str> {
        [&self.minutes, &self.hours, &self.days]
            .into_iter()
            .flat_map(|counted| [counted.one, counted.few, counted.many])
            .collect()
    }
}

pub fn lines(language: Interface) -> &'static Lines {
    match language {
        Interface::English => &ENGLISH,
        Interface::Spanish => &SPANISH,
        Interface::Portuguese => &PORTUGUESE,
        Interface::French => &FRENCH,
        Interface::German => &GERMAN,
        Interface::Italian => &ITALIAN,
        Interface::Catalan => &CATALAN,
        Interface::Galician => &GALICIAN,
        Interface::Basque => &BASQUE,
        Interface::Dutch => &DUTCH,
        Interface::Polish => &POLISH,
        Interface::Swedish => &SWEDISH,
    }
}

const ENGLISH: Lines = Lines {
    reading: "{name} is reading your notes...",
    adopted_none: "{name} found nothing worth keeping.",
    adopted: Counted::same("{name} kept 1 memory.", "{name} kept {count} memories."),
    listening: "{name} will be listening in {agent}.",
    available: "{name} is available in {agent}.",
    idle: "{name} has nothing to do.",
    watching: "{name} is watching over {memories} memories across {projects} projects",
    empty: "{name} has nothing to look after yet.",
    remembers: Counted::same(
        "{name} remembers 1 memory here.",
        "{name} remembers {count} memories here.",
    ),
    due: Counted::same(
        "{name} has 1 memory due for a reread. Open the queue with mem_review.",
        "{name} has {count} memories due for a reread. Open the queue with mem_review.",
    ),
    restored: Counted::same(
        "{name} held the thread: 1 memory restored.",
        "{name} held the thread: {count} memories restored.",
    ),
    recalls: Counted::same(
        "{name} has a note that might fit.",
        "{name} has {count} notes that might fit.",
    ),
    captured: Counted::same(
        "{name} kept 1 memory from that subagent.",
        "{name} kept {count} memories from that subagent.",
    ),
    nudge: "{name} has been given nothing to keep for {project} in {span}. If a decision was \
            made, a bug was fixed, or something non-obvious was learned, call mem_save now.",
    minutes: Counted::same("a minute", "{count} minutes"),
    hours: Counted::same("an hour", "{count} hours"),
    days: Counted::same("a day", "{count} days"),
};

const SPANISH: Lines = Lines {
    reading: "{name} está leyendo tus notas...",
    adopted_none: "{name} no encontró nada que valiera la pena guardar.",
    adopted: Counted::same(
        "{name} guardó 1 memoria.",
        "{name} guardó {count} memorias.",
    ),
    listening: "{name} estará escuchando en {agent}.",
    available: "{name} está disponible en {agent}.",
    idle: "{name} no tiene nada que hacer.",
    watching: "{name} cuida {memories} memorias repartidas en {projects} proyectos",
    empty: "{name} todavía no tiene nada que cuidar.",
    remembers: Counted::same(
        "{name} recuerda 1 memoria de aquí.",
        "{name} recuerda {count} memorias de aquí.",
    ),
    due: Counted::same(
        "{name} tiene 1 memoria que toca releer. Abre la cola con mem_review.",
        "{name} tiene {count} memorias que tocan releer. Abre la cola con mem_review.",
    ),
    restored: Counted::same(
        "{name} mantuvo el hilo: 1 memoria recuperada.",
        "{name} mantuvo el hilo: {count} memorias recuperadas.",
    ),
    recalls: Counted::same(
        "{name} tiene una nota que podría encajar.",
        "{name} tiene {count} notas que podrían encajar.",
    ),
    captured: Counted::same(
        "{name} guardó 1 memoria de ese subagente.",
        "{name} guardó {count} memorias de ese subagente.",
    ),
    nudge: "a {name} no le han dado nada que guardar para {project} en {span}. Si se tomó una \
            decisión, se corrigió un fallo o se aprendió algo que no era evidente, llama a \
            mem_save ahora.",
    minutes: Counted::same("un minuto", "{count} minutos"),
    hours: Counted::same("una hora", "{count} horas"),
    days: Counted::same("un día", "{count} días"),
};

const PORTUGUESE: Lines = Lines {
    reading: "{name} está a ler as tuas notas...",
    adopted_none: "{name} não encontrou nada que valesse a pena guardar.",
    adopted: Counted::same(
        "{name} guardou 1 memória.",
        "{name} guardou {count} memórias.",
    ),
    listening: "{name} estará à escuta em {agent}.",
    available: "{name} está disponível em {agent}.",
    idle: "{name} não tem nada que fazer.",
    watching: "{name} cuida de {memories} memórias espalhadas por {projects} projetos",
    empty: "{name} ainda não tem nada de que cuidar.",
    remembers: Counted::same(
        "{name} lembra-se de 1 memória daqui.",
        "{name} lembra-se de {count} memórias daqui.",
    ),
    due: Counted::same(
        "{name} tem 1 memória para reler. Abre a fila com mem_review.",
        "{name} tem {count} memórias para reler. Abre a fila com mem_review.",
    ),
    restored: Counted::same(
        "{name} segurou o fio: 1 memória recuperada.",
        "{name} segurou o fio: {count} memórias recuperadas.",
    ),
    recalls: Counted::same(
        "{name} tem uma nota que talvez encaixe.",
        "{name} tem {count} notas que talvez encaixem.",
    ),
    captured: Counted::same(
        "{name} guardou 1 memória desse subagente.",
        "{name} guardou {count} memórias desse subagente.",
    ),
    nudge: "a {name} não deram nada para guardar em {project} há {span}. Se se tomou uma \
            decisão, se corrigiu um erro ou se aprendeu algo que não era óbvio, chama mem_save \
            agora.",
    minutes: Counted::same("um minuto", "{count} minutos"),
    hours: Counted::same("uma hora", "{count} horas"),
    days: Counted::same("um dia", "{count} dias"),
};

const FRENCH: Lines = Lines {
    reading: "{name} lit tes notes...",
    adopted_none: "{name} n'a rien trouvé qui vaille la peine d'être gardé.",
    adopted: Counted::same(
        "{name} a gardé 1 souvenir.",
        "{name} a gardé {count} souvenirs.",
    ),
    listening: "{name} sera à l'écoute dans {agent}.",
    available: "{name} est disponible dans {agent}.",
    idle: "{name} n'a rien à faire.",
    watching: "{name} veille sur {memories} souvenirs répartis dans {projects} projets",
    empty: "{name} n'a encore rien à garder.",
    remembers: Counted::same(
        "{name} se souvient d'1 souvenir d'ici.",
        "{name} se souvient de {count} souvenirs d'ici.",
    ),
    due: Counted::same(
        "{name} a 1 souvenir à relire. Ouvre la file avec mem_review.",
        "{name} a {count} souvenirs à relire. Ouvre la file avec mem_review.",
    ),
    restored: Counted::same(
        "{name} a tenu le fil : 1 souvenir récupéré.",
        "{name} a tenu le fil : {count} souvenirs récupérés.",
    ),
    recalls: Counted::same(
        "{name} a une note qui pourrait convenir.",
        "{name} a {count} notes qui pourraient convenir.",
    ),
    captured: Counted::same(
        "{name} a gardé 1 souvenir de ce sous-agent.",
        "{name} a gardé {count} souvenirs de ce sous-agent.",
    ),
    nudge: "on n'a rien donné à garder à {name} pour {project} depuis {span}. Si une décision a \
            été prise, un bogue corrigé, ou quelque chose de non évident appris, appelle \
            mem_save maintenant.",
    minutes: Counted::same("une minute", "{count} minutes"),
    hours: Counted::same("une heure", "{count} heures"),
    days: Counted::same("un jour", "{count} jours"),
};

const GERMAN: Lines = Lines {
    reading: "{name} liest deine Notizen...",
    adopted_none: "{name} hat nichts gefunden, was zu behalten wäre.",
    adopted: Counted::same(
        "{name} hat 1 Erinnerung behalten.",
        "{name} hat {count} Erinnerungen behalten.",
    ),
    listening: "{name} wird in {agent} zuhören.",
    available: "{name} steht in {agent} bereit.",
    idle: "{name} hat nichts zu tun.",
    watching: "{name} wacht über {memories} Erinnerungen in {projects} Projekten",
    empty: "{name} hat noch nichts zu hüten.",
    remembers: Counted::same(
        "{name} erinnert sich hier an 1 Erinnerung.",
        "{name} erinnert sich hier an {count} Erinnerungen.",
    ),
    due: Counted::same(
        "{name} hat 1 Erinnerung zum Nachlesen fällig. Öffne die Liste mit mem_review.",
        "{name} hat {count} Erinnerungen zum Nachlesen fällig. Öffne die Liste mit mem_review.",
    ),
    restored: Counted::same(
        "{name} hat den Faden gehalten: 1 Erinnerung zurückgeholt.",
        "{name} hat den Faden gehalten: {count} Erinnerungen zurückgeholt.",
    ),
    recalls: Counted::same(
        "{name} hat eine Notiz, die passen könnte.",
        "{name} hat {count} Notizen, die passen könnten.",
    ),
    captured: Counted::same(
        "{name} hat 1 Erinnerung von diesem Subagenten behalten.",
        "{name} hat {count} Erinnerungen von diesem Subagenten behalten.",
    ),
    nudge: "{name} hat für {project} seit {span} nichts zu behalten bekommen. Wurde eine \
            Entscheidung getroffen, ein Fehler behoben oder etwas Unoffensichtliches gelernt, \
            rufe jetzt mem_save auf.",
    minutes: Counted::same("einer Minute", "{count} Minuten"),
    hours: Counted::same("einer Stunde", "{count} Stunden"),
    days: Counted::same("einem Tag", "{count} Tagen"),
};

const ITALIAN: Lines = Lines {
    reading: "{name} sta leggendo i tuoi appunti...",
    adopted_none: "{name} non ha trovato nulla che valesse la pena tenere.",
    adopted: Counted::same(
        "{name} ha tenuto 1 memoria.",
        "{name} ha tenuto {count} memorie.",
    ),
    listening: "{name} starà in ascolto in {agent}.",
    available: "{name} è disponibile in {agent}.",
    idle: "{name} non ha nulla da fare.",
    watching: "{name} veglia su {memories} memorie sparse in {projects} progetti",
    empty: "{name} non ha ancora nulla di cui occuparsi.",
    remembers: Counted::same(
        "{name} ricorda 1 memoria di qui.",
        "{name} ricorda {count} memorie di qui.",
    ),
    due: Counted::same(
        "{name} ha 1 memoria da rileggere. Apri la coda con mem_review.",
        "{name} ha {count} memorie da rileggere. Apri la coda con mem_review.",
    ),
    restored: Counted::same(
        "{name} ha tenuto il filo: 1 memoria recuperata.",
        "{name} ha tenuto il filo: {count} memorie recuperate.",
    ),
    recalls: Counted::same(
        "{name} ha un appunto che potrebbe calzare.",
        "{name} ha {count} appunti che potrebbero calzare.",
    ),
    captured: Counted::same(
        "{name} ha tenuto 1 memoria di quel subagente.",
        "{name} ha tenuto {count} memorie di quel subagente.",
    ),
    nudge: "a {name} non hanno dato nulla da tenere per {project} da {span}. Se è stata presa \
            una decisione, corretto un errore o imparato qualcosa di non ovvio, chiama mem_save \
            adesso.",
    minutes: Counted::same("un minuto", "{count} minuti"),
    hours: Counted::same("un'ora", "{count} ore"),
    days: Counted::same("un giorno", "{count} giorni"),
};

const CATALAN: Lines = Lines {
    reading: "{name} està llegint les teves notes...",
    adopted_none: "{name} no ha trobat res que valgués la pena guardar.",
    adopted: Counted::same(
        "{name} ha guardat 1 memòria.",
        "{name} ha guardat {count} memòries.",
    ),
    listening: "{name} estarà escoltant a {agent}.",
    available: "{name} està disponible a {agent}.",
    idle: "{name} no té res a fer.",
    watching: "{name} té cura de {memories} memòries repartides en {projects} projectes",
    empty: "{name} encara no té res a cuidar.",
    remembers: Counted::same(
        "{name} recorda 1 memòria d'aquí.",
        "{name} recorda {count} memòries d'aquí.",
    ),
    due: Counted::same(
        "{name} té 1 memòria per rellegir. Obre la cua amb mem_review.",
        "{name} té {count} memòries per rellegir. Obre la cua amb mem_review.",
    ),
    restored: Counted::same(
        "{name} ha mantingut el fil: 1 memòria recuperada.",
        "{name} ha mantingut el fil: {count} memòries recuperades.",
    ),
    recalls: Counted::same(
        "{name} té una nota que podria encaixar.",
        "{name} té {count} notes que podrien encaixar.",
    ),
    captured: Counted::same(
        "{name} ha guardat 1 memòria d'aquest subagent.",
        "{name} ha guardat {count} memòries d'aquest subagent.",
    ),
    nudge: "a {name} no li han donat res per guardar de {project} en {span}. Si s'ha pres una \
            decisió, s'ha corregit un error o s'ha après alguna cosa que no era evident, crida \
            mem_save ara.",
    minutes: Counted::same("un minut", "{count} minuts"),
    hours: Counted::same("una hora", "{count} hores"),
    days: Counted::same("un dia", "{count} dies"),
};

const GALICIAN: Lines = Lines {
    reading: "{name} está a ler as túas notas...",
    adopted_none: "{name} non atopou nada que pagase a pena gardar.",
    adopted: Counted::same(
        "{name} gardou 1 memoria.",
        "{name} gardou {count} memorias.",
    ),
    listening: "{name} estará a escoitar en {agent}.",
    available: "{name} está dispoñible en {agent}.",
    idle: "{name} non ten nada que facer.",
    watching: "{name} coida {memories} memorias repartidas en {projects} proxectos",
    empty: "{name} aínda non ten nada que coidar.",
    remembers: Counted::same(
        "{name} lembra 1 memoria de aquí.",
        "{name} lembra {count} memorias de aquí.",
    ),
    due: Counted::same(
        "{name} ten 1 memoria para reler. Abre a cola con mem_review.",
        "{name} ten {count} memorias para reler. Abre a cola con mem_review.",
    ),
    restored: Counted::same(
        "{name} mantivo o fío: 1 memoria recuperada.",
        "{name} mantivo o fío: {count} memorias recuperadas.",
    ),
    recalls: Counted::same(
        "{name} ten unha nota que podería encaixar.",
        "{name} ten {count} notas que poderían encaixar.",
    ),
    captured: Counted::same(
        "{name} gardou 1 memoria dese subaxente.",
        "{name} gardou {count} memorias dese subaxente.",
    ),
    nudge: "a {name} non lle deron nada que gardar para {project} en {span}. Se se tomou unha \
            decisión, se corrixiu un fallo ou se aprendeu algo que non era evidente, chama a \
            mem_save agora.",
    minutes: Counted::same("un minuto", "{count} minutos"),
    hours: Counted::same("unha hora", "{count} horas"),
    days: Counted::same("un día", "{count} días"),
};

const BASQUE: Lines = Lines {
    reading: "{name} zure oharrak irakurtzen ari da...",
    adopted_none: "{name}k ez du gordetzea merezi zuen ezer aurkitu.",
    adopted: Counted::same(
        "{name}k memoria 1 gorde du.",
        "{name}k {count} memoria gorde ditu.",
    ),
    listening: "{name} {agent} agentean entzuten egongo da.",
    available: "{name} {agent} agentean eskuragarri dago.",
    idle: "{name}k ez du zeregin bakar bat ere.",
    watching: "{name}k {memories} memoria zaintzen ditu, {projects} proiektutan banatuta",
    empty: "{name}k oraindik ez du zer zaindurik.",
    remembers: Counted::same(
        "{name}k hemengo memoria 1 gogoratzen du.",
        "{name}k hemengo {count} memoria gogoratzen ditu.",
    ),
    due: Counted::same(
        "{name}k berrirakurtzeko memoria 1 du. Ireki ilara mem_review erabiliz.",
        "{name}k berrirakurtzeko {count} memoria ditu. Ireki ilara mem_review erabiliz.",
    ),
    restored: Counted::same(
        "{name}k haria eutsi du: memoria 1 berreskuratuta.",
        "{name}k haria eutsi du: {count} memoria berreskuratuta.",
    ),
    recalls: Counted::same(
        "{name}k bat etor litekeen ohar bat du.",
        "{name}k bat etor litezkeen {count} ohar ditu.",
    ),
    captured: Counted::same(
        "{name}k azpiagente horren memoria 1 gorde du.",
        "{name}k azpiagente horren {count} memoria gorde ditu.",
    ),
    nudge: "{name}ri ez diote {project} proiekturako ezer gordetzeko eman {span}. \
            Erabaki bat hartu bada, akats bat konpondu bada, edo agerikoa ez zen zerbait ikasi \
            bada, deitu mem_save orain.",
    minutes: Counted::same("minutu batean", "{count} minutuan"),
    hours: Counted::same("ordu batean", "{count} ordutan"),
    days: Counted::same("egun batean", "{count} egunetan"),
};

const DUTCH: Lines = Lines {
    reading: "{name} leest je aantekeningen...",
    adopted_none: "{name} heeft niets gevonden dat het bewaren waard was.",
    adopted: Counted::same(
        "{name} heeft 1 herinnering bewaard.",
        "{name} heeft {count} herinneringen bewaard.",
    ),
    listening: "{name} luistert mee in {agent}.",
    available: "{name} staat klaar in {agent}.",
    idle: "{name} heeft niets te doen.",
    watching: "{name} waakt over {memories} herinneringen verspreid over {projects} projecten",
    empty: "{name} heeft nog niets om voor te zorgen.",
    remembers: Counted::same(
        "{name} herinnert zich hier 1 herinnering.",
        "{name} herinnert zich hier {count} herinneringen.",
    ),
    due: Counted::same(
        "{name} heeft 1 herinnering om te herlezen. Open de wachtrij met mem_review.",
        "{name} heeft {count} herinneringen om te herlezen. Open de wachtrij met mem_review.",
    ),
    restored: Counted::same(
        "{name} hield de draad vast: 1 herinnering teruggehaald.",
        "{name} hield de draad vast: {count} herinneringen teruggehaald.",
    ),
    recalls: Counted::same(
        "{name} heeft een aantekening die zou kunnen passen.",
        "{name} heeft {count} aantekeningen die zouden kunnen passen.",
    ),
    captured: Counted::same(
        "{name} heeft 1 herinnering van die subagent bewaard.",
        "{name} heeft {count} herinneringen van die subagent bewaard.",
    ),
    nudge: "{name} heeft in {span} niets te bewaren gekregen voor {project}. Is er een besluit \
            genomen, een fout hersteld, of iets niet vanzelfsprekends geleerd, roep dan nu \
            mem_save aan.",
    minutes: Counted::same("een minuut", "{count} minuten"),
    hours: Counted::same("een uur", "{count} uur"),
    days: Counted::same("een dag", "{count} dagen"),
};

const POLISH: Lines = Lines {
    reading: "{name} czyta twoje notatki...",
    adopted_none: "{name} nie znalazł nic wartego zachowania.",
    adopted: Counted {
        one: "{name} zachował 1 wspomnienie.",
        few: "{name} zachował {count} wspomnienia.",
        many: "{name} zachował {count} wspomnień.",
    },
    listening: "{name} będzie słuchać w {agent}.",
    available: "{name} jest dostępny w {agent}.",
    idle: "{name} nie ma nic do roboty.",
    watching: "{name} czuwa nad {memories} wspomnieniami w {projects} projektach",
    empty: "{name} nie ma jeszcze czym się opiekować.",
    remembers: Counted {
        one: "{name} pamięta stąd 1 wspomnienie.",
        few: "{name} pamięta stąd {count} wspomnienia.",
        many: "{name} pamięta stąd {count} wspomnień.",
    },
    due: Counted {
        one: "{name} ma 1 wspomnienie do ponownego przeczytania. Otwórz kolejkę przez \
              mem_review.",
        few: "{name} ma {count} wspomnienia do ponownego przeczytania. Otwórz kolejkę przez \
              mem_review.",
        many: "{name} ma {count} wspomnień do ponownego przeczytania. Otwórz kolejkę przez \
               mem_review.",
    },
    restored: Counted {
        one: "{name} utrzymał wątek: 1 wspomnienie odzyskane.",
        few: "{name} utrzymał wątek: {count} wspomnienia odzyskane.",
        many: "{name} utrzymał wątek: {count} wspomnień odzyskanych.",
    },
    recalls: Counted {
        one: "{name} ma notatkę, która może pasować.",
        few: "{name} ma {count} notatki, które mogą pasować.",
        many: "{name} ma {count} notatek, które mogą pasować.",
    },
    captured: Counted {
        one: "{name} zachował 1 wspomnienie od tego subagenta.",
        few: "{name} zachował {count} wspomnienia od tego subagenta.",
        many: "{name} zachował {count} wspomnień od tego subagenta.",
    },
    nudge: "{name} nie dostał nic do zachowania dla {project} od {span}. Jeśli podjęto decyzję, \
            naprawiono błąd albo nauczono się czegoś nieoczywistego, wywołaj teraz mem_save.",
    minutes: Counted {
        one: "minuty",
        few: "{count} minut",
        many: "{count} minut",
    },
    hours: Counted {
        one: "godziny",
        few: "{count} godziny",
        many: "{count} godzin",
    },
    days: Counted {
        one: "dnia",
        few: "{count} dni",
        many: "{count} dni",
    },
};

const SWEDISH: Lines = Lines {
    reading: "{name} läser dina anteckningar...",
    adopted_none: "{name} hittade inget värt att behålla.",
    adopted: Counted::same("{name} behöll 1 minne.", "{name} behöll {count} minnen."),
    listening: "{name} kommer att lyssna i {agent}.",
    available: "{name} finns till hands i {agent}.",
    idle: "{name} har inget att göra.",
    watching: "{name} vakar över {memories} minnen spridda på {projects} projekt",
    empty: "{name} har inget att se efter än.",
    remembers: Counted::same(
        "{name} minns 1 minne härifrån.",
        "{name} minns {count} minnen härifrån.",
    ),
    due: Counted::same(
        "{name} har 1 minne att läsa om. Öppna kön med mem_review.",
        "{name} har {count} minnen att läsa om. Öppna kön med mem_review.",
    ),
    restored: Counted::same(
        "{name} höll tråden: 1 minne återhämtat.",
        "{name} höll tråden: {count} minnen återhämtade.",
    ),
    recalls: Counted::same(
        "{name} har en anteckning som kan passa.",
        "{name} har {count} anteckningar som kan passa.",
    ),
    captured: Counted::same(
        "{name} behöll 1 minne från den underagenten.",
        "{name} behöll {count} minnen från den underagenten.",
    ),
    nudge: "{name} har inte fått något att behålla för {project} på {span}. Om ett beslut togs, \
            ett fel rättades, eller något icke självklart lärdes, anropa mem_save nu.",
    minutes: Counted::same("en minut", "{count} minuter"),
    hours: Counted::same("en timme", "{count} timmar"),
    days: Counted::same("en dag", "{count} dagar"),
};
