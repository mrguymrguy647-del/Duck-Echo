"""The Ear's own words: about 100 kinds of game sound and music.

Each is (the short word shown to agents, the sentence the model hears).
CLAP2's authors advise full sentences in the style of its training
captions, so the model hears "A coin is collected in a video game.", and
agents read "coin pickup".

They are the other side of every `match`: a text's match says how much
more it fits a sound than these do. Changing this list changes every
match, so `WORDS_VERSION` is reported in `hello`, and the answer cache
keys on it.
"""

WORDS_VERSION = "1"

WORDS = [
    # Pickups and rewards
    ("coin pickup", "A coin is collected in a video game."),
    ("gem pickup", "A sparkling gem is picked up in a game."),
    ("power-up", "A rising power-up jingle in a video game."),
    ("extra life", "A cheerful extra life sound in a retro game."),
    ("key pickup", "A small key is picked up with a jingle."),
    ("level complete fanfare", "A short victory fanfare plays."),
    ("bell ding", "A single bell rings with a clear ding."),
    ("chime", "A soft chime rings."),
    # Interface
    ("ui click", "A short click of a menu button."),
    ("ui confirm", "A short pleasant beep confirms a choice in a menu."),
    ("ui cancel", "A short low beep cancels a menu choice."),
    ("ui error", "A buzzing error beep."),
    ("menu open", "A menu opens with a soft whoosh."),
    ("typing", "Someone types on a keyboard."),
    ("notification", "A phone notification sound."),
    # Movement
    ("jump", "A character jumps with a springy boing."),
    ("landing thud", "A character lands on the ground with a thud."),
    ("footsteps", "Footsteps walking on a hard floor."),
    ("footsteps on gravel", "Footsteps crunch on gravel."),
    ("footsteps on grass", "Soft footsteps on grass."),
    ("running", "Someone runs quickly."),
    ("whoosh", "A fast whoosh of air."),
    ("swoosh of a sword", "A sword swings through the air."),
    # Combat
    ("punch", "A heavy punch hits someone."),
    ("hit", "A sharp hit sound in a fighting game."),
    ("sword clash", "Two metal swords clash."),
    ("metal clang", "A metal object clangs loudly."),
    ("laser shot", "A laser gun fires a zap."),
    ("blaster", "A sci-fi blaster fires."),
    ("gunshot", "A gun fires a single shot."),
    ("shotgun", "A shotgun blast."),
    ("missile", "A missile launches with a roar."),
    ("explosion", "A loud explosion with a deep boom."),
    ("big explosion", "A huge explosion rumbles for a long time."),
    ("small explosion", "A small pop of an explosion."),
    ("hurt", "A character gets hurt with a short grunt."),
    ("shield", "A magic shield hums."),
    # Magic and sci-fi
    ("magic spell", "A sparkling magic spell is cast."),
    ("teleport", "A shimmering teleport sound."),
    ("charge up", "A sound charges up, rising in pitch."),
    ("power down", "A machine powers down, falling in pitch."),
    ("robot", "Robotic beeps and whirs."),
    ("computer beeps", "A computer makes beeping sounds."),
    ("alarm", "A loud alarm siren wails."),
    ("warning beep", "A repeating warning beep."),
    ("low health warning", "A heartbeat-like warning when health is low."),
    ("siren", "A police siren."),
    ("radio static", "Radio static crackles."),
    # Machines and vehicles
    ("engine hum", "An engine hums steadily."),
    ("car engine", "A car engine revs."),
    ("motor", "An electric motor whirs."),
    ("spaceship", "A spaceship hums as it flies."),
    ("door open", "A door creaks open."),
    ("door slam", "A door slams shut."),
    ("switch", "A switch clicks."),
    ("lock", "A lock clicks open."),
    ("glass breaking", "Glass shatters."),
    ("wood breaking", "Wood cracks and breaks."),
    ("rock breaking", "Rocks crumble and break."),
    ("digging", "Someone digs in dirt with a shovel."),
    ("block placed", "A block is placed with a soft thud."),
    # Nature
    ("wind", "Wind blows."),
    ("rain", "Rain falls steadily."),
    ("thunder", "Thunder rumbles."),
    ("water splash", "Something splashes into water."),
    ("river", "A river flows."),
    ("fire crackling", "A fire crackles."),
    ("birds", "Birds sing."),
    ("duck quack", "A duck quacks."),
    ("dog bark", "A dog barks."),
    ("cat meow", "A cat meows."),
    ("insects", "Insects buzz and chirp at night."),
    # People
    ("speech", "A person speaks."),
    ("laughter", "People laugh."),
    ("applause", "People applaud."),
    ("crowd", "A crowd murmurs."),
    ("singing", "A person sings."),
    # Music
    ("calm music", "Calm, slow, peaceful music."),
    ("tense music", "Tense, suspenseful music."),
    ("happy music", "Happy, upbeat music."),
    ("sad music", "Sad, slow music."),
    ("scary music", "Scary, dark horror music."),
    ("exciting music", "Fast, exciting action music."),
    ("chiptune music", "Retro 8-bit chiptune video game music."),
    ("piano music", "Gentle piano music."),
    ("ambient pad", "A soft ambient synth pad."),
    ("drum beat", "A drum beat."),
    ("electronic dance music", "Electronic dance music with a strong beat."),
    ("orchestral music", "Orchestral music with strings."),
    ("music box", "A music box plays a tune."),
    ("bass", "A deep bass line."),
    ("guitar", "Someone plays a guitar."),
    # Signals and textures
    ("sine tone", "A pure steady sine tone."),
    ("beep", "A single electronic beep."),
    ("buzz", "A harsh electrical buzz."),
    ("hum", "A low steady hum."),
    ("white noise", "White noise hisses."),
    ("static hiss", "A steady static hiss."),
    ("silence", "Silence. Nothing can be heard."),
    ("click", "A single short click."),
    ("pop", "A short pop."),
    ("rumble", "A low rumble."),
]
