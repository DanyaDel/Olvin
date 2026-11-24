# Сеть

Сеть живёт в крейте `olvin_net` (фича `net` у `olvin`, `net-quic` для настоящих сокетов). Решение и обоснование: `docs/adr/0003-netcode.md` (ADR 0003). Цель: сеть очень гибкая и при этом несложная в написании. Она устроена слоями, каждый можно взять отдельно, а верхний слой занимает несколько строк.

## Быстрый старт

Хост или клиент из командной строки, реплицируемый префаб игрока, одна проверяемая команда, один Lua-скрипт. Это `apps/netdemo/examples/quickstart.rs`, он собирается вместе с репозиторием:

```rust
use olvin::core::{EventCursor, Events};
use olvin::net::*;
use olvin::prelude::*;
use olvin::script::lua::LuaPlugin;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Move(Vec2);

fn main() {
    let cfg = NetConfig::from_args("my-game", 1, std::env::args()).expect("--host | --join <code>");
    let mut app = App::new();
    app.add_plugin(HeadlessPlugins::default())
        .add_plugin(NetPlugin::new(cfg));
    app.add_plugin(LuaPlugin::new("assets/scripts"))
        .add_plugin(LuaNetPlugin);
    app.add_plugin(PlayerSpawner::new("prefabs/player.prefab.ron")); // a Replicated prefab per client
    app.replicate::<Transform>(); // positions travel and are interpolated
    app.net_command::<Move>("move", |_, m| {
        Verdict::check(m.0.length() <= 1.01, "too fast")
    });
    let mut seen = EventCursor::default();
    app.add_system(Stage::Update, move |res| {
        let moves = res.fetch::<Events<NetReceived<Move>>>();
        let mut world = res.fetch_mut::<World>();
        for m in moves.read(&mut seen) {
            for (o, t) in world.query_mut::<(&Owner, &mut Transform)>() {
                if Some(o.0) == m.from.client() {
                    t.translation += Vec3::new(m.msg.0.x, 0.0, m.msg.0.y) * 0.1;
                }
            }
        }
    });
    app.run();
}
```

Запустите `cargo run -p netdemo --example quickstart -- --host` и передайте напечатанный код комнаты в `--join <код>` на другой машине или в другом терминале. Без аргументов игра идёт offline: тот же код, сервер с одним локальным игроком.

Игрок это `apps/netdemo/assets/prefabs/player.prefab.ron`: обычный префаб с одной лишней записью, `components: {"Replicated": "()"}`. В редакторе та же запись добавляется любому объекту в инспекторе (Components), и объект становится сетевым без кода. Lua-скрипт это `assets/scripts/Player.lua`:

```lua
-- Runs on the server and on every client copy of a player.
-- The owner's client fires once a second; the server counts shots per player and
-- replicates the count, so every client can read it with ctx:var.
return {
  start = function(self, ctx)
    self.frame = 0
    if ctx:is_server() and ctx:owner() then
      ctx:replicate_var("shots_" .. ctx:owner())
    end
  end,
  update = function(self, ctx)
    self.frame = self.frame + 1
    if ctx:is_client() and not ctx:is_server() and ctx:is_mine() and self.frame % 60 == 0 then
      ctx:net_send("server", "fire", self.frame)
    end
  end,
  on_net_message = function(self, ctx, name, value, from)
    -- Every player script hears every message: only the sender's own counts.
    if ctx:is_server() and name == "fire" and from == ctx:owner() then
      local key = "shots_" .. from
      ctx:set_var(key, (ctx:var(key) or 0) + 1)
    end
  end,
}
```

`cargo run -p netdemo -- --selftest` запускает сервер и двух ботов-клиентов в одном процессе по QUIC на 127.0.0.1 и проверяет, что они согласны. `netdemo --host --frames 600` и `netdemo --join <код> --bot --frames 600` запускают их отдельными процессами.

## Слои

| Слой | Что | Главное API |
|---|---|---|
| L0 Транспорт | надёжный и ненадёжный каналы; `Loopback` внутри процесса с имитацией задержки, джиттера, потерь, дублей и перестановок; QUIC с TLS 1.3 | `Transport`, `Loopback`, `QuicServer`, `QuicClient` |
| L1 Сессия | `Offline`, `Host` (listen-сервер, хост играет), `Dedicated`, `Client`; рукопожатие с проверкой протокола, игры и реестра; вход, выход, кик, heartbeat, таймауты, токен переподключения; коды комнат; подключаемая аутентификация | `NetConfig`, `NetPlugin`, `Net`, `NetEvent`, `Authenticator`, `RoomCode` |
| L2 Репликация | мир ведёт сервер; сущности с `Replicated` появляются у клиентов тем же префабом; дельта-снимки с подтверждениями, интерполяция, интерес, бюджет трафика | `Replicated`, `Owner`, `NetId`, `app.replicate::<T>()`, `ReplicateOptions` |
| L3 Сообщения | любой serde-тип с именем; команды, которые проверяет сервер; лимиты частоты и размера | `app.net_message::<T>()`, `app.net_command::<T>()`, `net.send(Target, &msg)`, `NetReceived<T>`, `Verdict` |
| L4 Предсказание | персонаж владельца откликается сразу, сервер его поправляет | `Prediction<S, I>`, `net.predict_character(..)`, `PoseHistory` |
| L5 Lua | `ctx:is_server()`, `ctx:net_send`, `on_net_message`, реплицируемые переменные | см. [Lua API](../reference/lua-api.md) |
| L6 Инструменты | ресурс `NetStats` (RTT, трафик, потери, отставание по тикам), `Testbed` для тестов, `OLVIN_NET_SIM` | `NetStats`, `testing::Testbed` |

Каждый слой работает без верхних: игре может хватить `Loopback` и `Net::send`, или репликации без предсказания, или Lua без Rust-компонентов.

## Настройка

Одна структура, `NetConfig`: конструктор и цепочка настроек.

| Настройка | По умолчанию | Смысл |
|---|---|---|
| `NetConfig::offline / host / dedicated / join / from_args` | | режим; `from_args` читает `--host [адрес]`, `--dedicated [адрес]`, `--join <код или адрес>`, `--name`, `--net-sim` |
| `tick_rate(hz)` | 30 | тиков сервера в секунду |
| `interp_delay(s)` | 0.1 | на сколько в прошлом показываются чужие сущности |
| `max_clients(n)` | 16 | |
| `bandwidth(байт/с)` | 65536 | бюджет снимков на клиента |
| `timeout(s)` | 10 | тишина, после которой участника отключают |
| `player_name`, `token` | | уходят в рукопожатии; `Authenticator` сервера видит токен |
| `simulate(NetConditions)` | нет | наложить «лаг 80 мс, потери 2 %» на настоящий транспорт (`OLVIN_NET_SIM="lag=80,loss=2,jitter=10"`) |
| `allow_unpinned(bool)` | false | входить на голый `host:port` без закреплённого сертификата |
| `limits(NetLimits)` | | пределы пакетов, сообщений, имён и числа сущностей |

Id и версия игры у сервера и клиента должны совпадать, как и регистрации: компоненты и сообщения регистрируются по имени в одном и том же порядке везде (рукопожатие сравнивает хеш и отказывает при несовпадении, с понятным сообщением).

## Безопасность

- **Авторитет сервера.** Реплицируемые компоненты пишет только сервер. У клиента нет сообщения, которое задаёт состояние: он шлёт команды (`Move`, `Fire`), а сервер их проверяет. `Owner(ClientId)` говорит, кто управляет сущностью; смена владельца это запись на сервере.
- **Шифрование и личность.** QUIC даёт TLS 1.3. Сертификат хоста самоподписанный, а **код комнаты** (`OLV-…`) несёт адрес хоста и первые 16 байт SHA-256 сертификата: клиент принимает только этот сертификат. Голый `host:port` отвергается без `allow_unpinned(true)`.
- **Ключи сессии и повторы.** После рукопожатия каждый пакет несёт номер и усечённый HMAC-SHA256 на ключе из nonce обеих сторон и экспортёра TLS; дубли и старые пакеты отбрасываются. В рукопожатии есть nonce и окно меток времени (60 с), так что записанный `Hello` не воспроизвести.
- **Недоверенный ввод.** Размеры пакетов и сообщений, частота на клиента и на тип сообщения, имена, число сущностей и буферы ограничены; мусорные байты не роняют (тесты кормят мусором каждый разборщик); клиент, который продолжает нарушать, кикается. Клиент тоже отвергает путь префаба от сервера, если это не относительный `*.prefab.ron`.
- **Серверных секретов в клиенте нет.** Ключ сертификата (`QuicIdentity`) и аутентификаторы живут только в серверных путях кода. Называйте серверные секреты `server_*`; CI-проверка этого в планах.

## Предсказание

`net.predict_character(sample)` предсказывает персонажа клиента на `move_character`; `Prediction<S, I>` делает то же для любого состояния и ввода. Клиент сразу применяет ввод, шлёт последние вводы с номерами, а сервер применяет ровно один ввод за тик. Состояние сервера несёт «последний применённый мной ввод»; если оно отличается от предсказанного клиентом больше порога, клиент сдвигается на долю пути к состоянию сервера и заново применяет неподтверждённые вводы (большая ошибка устраняется сразу). Сервер ограничивает вводы (`sanitize`), и ускориться заливом пакетов нельзя. Создавайте игрока с `CharacterState::at(позиция)` и тем же `Transform`.

## Тесты сетевой игры

`olvin_net::testing::Testbed` гоняет сервер и клиентов отдельными `App` в одном процессе через `Loopback` с выбранными условиями; тест шагает всеми с фиксированными часами, так что запуски быстрые и повторяемые:

```rust
let mut tb = Testbed::new(NetConditions::lag_ms(80.0).with_loss_percent(2.0));
tb.add_server_with(NetConfig::dedicated("my-game", 1, "x"), register_game);
tb.add_client_with(NetConfig::join("my-game", 1, "1.1.1.1:1")?, register_game);
tb.run_until(5.0, |tb| tb.net(1).status() == NetStatus::Connected);
```

## Границы

Нет: подбора игроков и лобби на сервере, пробивки NAT и ретрансляторов, отката для файтингов, lockstep, голоса, шардинга на сотни игроков, античита сверх авторитета сервера и проверки. Чужие копии несут компоненты самого префаба: не давайте им собственного физического тела (запрос `Without<Remote>`). В редакторе пока нет настройки «Play с лагом»; используйте `OLVIN_NET_SIM` или `NetConfig::simulate`.

## См. также

- [Время и события](time-events.md)
- [Скрипты](scripting.md)
- [Префабы](prefabs.md)
- [Состояния и сохранения](game-states-saves.md)
