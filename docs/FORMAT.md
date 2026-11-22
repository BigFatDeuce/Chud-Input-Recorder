# Format de fichier `.chud`

> Référence du format ouvert de Chud Input Recorder. L'implémentation fait foi :
> `crates/chud-core/src/format.rs` (archive, codec, export JSON), `event.rs` (événements),
> `document.rs` (métadonnées), `flow.rs` (automatisation).
> Version actuelle du format : **1** (`FORMAT_VERSION`). Codec d'événements : **CHEV v1**.

## 1. L'archive

Un fichier `.chud` est une archive **ZIP** standard (ouvrable avec n'importe quel outil) :

| Entrée | Obligatoire | Contenu |
|---|---|---|
| `manifest.json` | oui | format, version, métadonnées, réglages de lecture, environnement, marqueurs, résumé |
| `events.bin` | oui | prise courante, codec binaire CHEV (§3) |
| `original.bin` | non | prise originale intacte (édition non destructive), même codec |
| `contexts.json` | non | changements de fenêtre active pendant l'enregistrement |
| `flow.json` | non | blocs d'automatisation (§5) |
| `images/<id>.png` | non | images cherchées à l'écran par le flux (§6), stockées sans compression ZIP |

Les entrées sont compressées (Deflate), sauf les images PNG, déjà compressées. L'enregistrement est **atomique** : le fichier est
écrit à côté puis renommé par-dessus l'ancien, un plantage ne laisse jamais de macro à moitié
écrite. Un fichier d'une version plus récente est refusé avec un message clair
(`TooNew`) plutôt que lu de travers.

## 2. `manifest.json`

```json
{
  "format": "chud-input-recorder",
  "version": 1,
  "meta": {
    "id": "uuid", "name": "…", "description": "…", "tags": ["…"],
    "icon": "…", "color": "…", "favorite": false,
    "created_ms": 0, "modified_ms": 0, "app_version": "0.1.0", "source_os": "windows"
  },
  "settings": { "speed": 1.0, "repeat": 1, "timing": "exact", "mouse": "absolute", "…": "…" },
  "env": { "monitors": [ { "name": "…", "rect": { "x": 0, "y": 0, "width": 2560, "height": 1440 }, "scale": 1.75, "primary": true, "refresh_hz": 165 } ], "…": "…" },
  "markers": [ { "…": "…" } ],
  "summary": {
    "event_count": 265, "duration_us": 2050000,
    "keyboard": 12, "mouse_buttons": 4, "mouse_moves": 240, "wheel": 9, "gamepad": 0,
    "has_flow": false, "density": [0, 12, 255, "… 48 valeurs 0..255"]
  },
  "has_original": true
}
```

Le `summary` permet à la bibliothèque d'afficher une macro (durée, aperçu de densité) sans
décoder ses événements. Les champs inconnus sont ignorés et les champs absents prennent leur
valeur par défaut : un ancien lecteur ouvre un fichier plus riche, et inversement.

## 3. Codec d'événements CHEV v1 (`events.bin`, `original.bin`)

```
"CHEV"            4 octets (magic)
version           1 octet  (= 1)
n                 uvar     (nombre d'événements)
n × événement :
  dt              uvar     microsecondes depuis l'événement précédent (le premier : depuis 0)
  tag             1 octet
  charge utile    selon le tag
```

- **uvar** : entier non signé LEB128 (7 bits par octet, bit de poids fort = « suite »).
- **ivar** : entier signé en zig-zag (`(v << 1) ^ (v >> 63)`) puis LEB128.
- **pos** : position absolue codée en **delta** par rapport à la dernière position connue
  (`ivar dx`, `ivar dy`), partagée par tous les tags qui portent une position. Départ (0, 0).

| Tag | Événement | Charge utile |
|---|---|---|
| 1 | `mouse_move` | pos |
| 2 | `mouse_raw` (delta du capteur) | ivar dx, ivar dy |
| 3 / 4 | `mouse_down` / `mouse_up` | octet bouton, pos |
| 5 | `wheel` | octet horizontal (0/1), ivar delta, pos |
| 6 / 10 / 7 | `key_down` / `key_down` répétition auto / `key_up` | uvar scan, uvar vk |
| 8 | `pad_button` | octet manette, octet bouton, octet enfoncé (0/1) |
| 9 | `pad_axis` | octet manette, octet axe, ivar valeur |

Valeurs :

- Temps : microsecondes, `i64`, depuis le début de la prise. Les événements sont triés par temps.
- Positions : pixels **physiques** du bureau virtuel (peuvent être négatives : écran à gauche
  ou au-dessus du principal).
- Boutons de souris : 0 gauche, 1 droit, 2 milieu, 3 X1 (précédent), 4 X2 (suivant).
- Molette : unités `WHEEL_DELTA` (120 = un cran), négatif = vers soi / vers la gauche.
- Touches : `scan` = code **set 1** PC/AT, indépendant de la disposition (touches étendues avec
  `0xE0` dans l'octet haut, ex. Ctrl droit `0xE01D` ; codes evdev sans équivalent PC :
  `0xF000 | code`) ; `vk` = code virtuel Windows observé (la touche *logique*), 0 si inconnu.
- Manettes : boutons 0 A/Croix, 1 B/Rond, 2 X/Carré, 3 Y/Triangle, 4 Back, 5 Guide, 6 Start,
  7 LS, 8 RS, 9 LB, 10 RB, 11–14 croix directionnelle (haut, bas, gauche, droite), 15 Misc,
  16–19 palettes, 20 pavé tactile ; boutons bruts à partir de 64. Axes 0/1 stick gauche X/Y,
  2/3 stick droit X/Y (−32768..32767, Y vers le bas), 4/5 gâchettes (0..32767) ; axes bruts à
  partir de 64.

Le décodage est **exact** (aller-retour testé) et refuse les flux corrompus (fin inattendue,
tag inconnu, valeurs hors limites) au lieu de produire des événements faux.

## 4. Export JSON intégral

`Exporter en JSON` (ou `chud export fichier.chud sortie.json`) produit un document autonome :

```json
{
  "format": "chud-input-recorder", "version": 1,
  "meta": { "…": "…" }, "settings": { "…": "…" }, "env": { "…": "…" },
  "markers": [], "contexts": [], "flow": { "blocks": [] },
  "images": { "a1b2c3d4": "iVBORw0KGgo… (PNG en base64)" },
  "events": [
    { "t": 0, "type": "mouse_move", "x": 812, "y": 404 },
    { "t": 15230, "type": "mouse_down", "button": "left", "x": 812, "y": 404 },
    { "t": 90112, "type": "key_down", "key": { "scan": 30, "vk": 65 } },
    { "t": 151004, "type": "key_up", "key": { "scan": 30, "vk": 65 } }
  ]
}
```

Il se réimporte sans perte. `images` n'apparaît que si le flux contient des images.

## 5. `flow.json` (automatisation)

```json
{ "blocks": [ { "id": "8f3a91c2", "enabled": true, "type": "repeat", "count": "3", "counter": "i", "body": [ … ] } ] }
```

Chaque bloc a un `id`, `enabled` et un `type` ; ses champs propres sont au même niveau.
Les champs d'expression (`ms`, `count`, `x`, `y`, `value`, `expr`) sont des chaînes évaluées par
un langage d'expressions sûr (`random(200, 400)`, `i * 50 + 10`, `found && found_score > 0.95`).

| `type` | Champs |
|---|---|
| `play_recording` | `start`, `end` (µs ou `null`), `speed` |
| `play_macro` | `macro_id`, `name` |
| `wait` | `ms` (expression) |
| `keys` | `keys` (liste de touches), `hold_ms` |
| `key_down`, `key_up` | `key` |
| `type_text` | `text`, `interval_ms` |
| `move_mouse` | `x`, `y` (expressions), `relative`, `duration_ms`, `motion` (`instant`, `linear`, `smooth`, `human`) |
| `click` | `button`, `count`, `interval_ms`, `hold_ms` |
| `mouse_down`, `mouse_up` | `button` |
| `scroll` | `notches`, `horizontal` |
| `repeat` | `count` (expression ou `null` = sans fin), `counter`, `body` |
| `while` | `cond`, `body`, `max_iterations` |
| `if` | `cond`, `then`, `otherwise` |
| `wait_until` | `cond`, `timeout_ms`, `poll_ms` |
| `find` | voir §6 |
| `set_var` | `name`, `value` |
| `notify` | `message` (`{variable}` est remplacé par sa valeur) |
| `break`, `continue`, `stop` | — |

Conditions (`cond`, champ `type`) : `expr`, `window_active` / `window_exists` (`title`,
`process`), `pixel_color` (`x`, `y`, `color`, `tolerance`), `key_held` (`key`), `on_screen`
(`target`, `region`), `chance` (`percent`), `not` (`cond`), `all` / `any` (`conds`).

## 6. Recherche à l'écran (`find`, `on_screen`, `images/`)

```json
{
  "type": "find",
  "target": { "kind": "image", "image": "a1b2c3d4", "threshold": 0.9, "any_scale": false },
  "region": { "x": 0, "y": 1356, "w": 2560, "h": 84 },
  "timeout_ms": 5000, "poll_ms": 250,
  "action": "click", "offset_x": 0, "offset_y": 0,
  "duration_ms": 250, "motion": "instant",
  "var": "found", "if_missing": "fail"
}
```

- `target` (champ `kind`) :
  - `image` : `image` = identifiant d'une entrée `images/<id>.png` ; `threshold` = ressemblance
    minimale 0..1 (défaut 0.9) ; `any_scale` = chercher aussi l'image plus petite ou plus grande.
  - `color` : `color` = `#rrggbb`, `tolerance` 0..255 par canal ; trouve le premier pixel en
    partant du coin haut gauche.
  - `text` : `text` (mot ou phrase), `match_case`. Lu par l'OCR intégré à Windows.
- `region` : zone de recherche en pixels physiques du bureau virtuel, `null` = tous les écrans.
- `timeout_ms` : 0 = un seul essai ; sinon recherche répétée toutes les `poll_ms` ms.
- `action` : `none` (seulement les variables), `move`, `click`, `double_click`, `right_click`,
  `middle_click`. Le point visé est le centre trouvé + (`offset_x`, `offset_y`).
- Variables définies : `<var>` (booléen), `<var>_x`, `<var>_y` (point visé), `<var>_w`,
  `<var>_h`, `<var>_score` (0..1). Si rien n'est trouvé, `<var>` vaut faux et les autres sont
  retirées.
- `if_missing` : `continue`, `stop` (fin silencieuse) ou `fail` (fin avec message d'erreur).

Identifiants d'image : 1 à 64 caractères `A–Z a–z 0–9 - _` ; un identifiant non conforme n'est
jamais écrit dans l'archive (pas de chemin qui sortirait du dossier `images/`). Les pixels
transparents (alpha < 128) d'une image importée sont ignorés pendant la recherche.
