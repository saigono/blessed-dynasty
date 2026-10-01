# Этап 0. Каркас и детерминизм

## Контекст
Прочитай CLAUDE.md и DESIGN.md (разделы 7.4 и 10). Это первый этап, репозиторий пуст.

## Вход
Ничего. Ты задаёшь базовые типы для всех остальных этапов.

## Сделать
- `Cargo.toml` workspace с крейтами `crates/core`, `crates/cli`, `crates/ui`. `cli` и `ui` пока печатают версию и выходят.
- `core/src/rng.rs`: xoshiro256** без внешних крейтов. API: `Rng::from_seed(u64)`, `next_u64()`, `range(lo: i64, hi: i64) -> i64` (включительно lo, исключительно hi), `chance(num: u32, den: u32) -> bool`. Сериализуемое состояние.
- `core/src/time.rs`: `TimeUnit { ticks_per_year: u32 }`, `Tick(u32)`, `Tick::year(&self, unit) -> u32`, `Years(u32) -> Tick` через unit, отображаемая дата `"весна 1187"` для `ticks_per_year` 1, 4, 12 (при 1 только год).
- `core/src/fx.rs`: `Fx(i64)` с масштабом 1000: `from_int`, `from_milli`, `+ - *` и деление с округлением к нулю, `clamp`, `Display`. `Serialize`/`Deserialize` как число.
- Загрузка данных: зависимости `serde`, `ron`. Функция `core::data::load(&str) -> Result<Data, DataError>`, где `Data` пока содержит только `time_unit`. Пример `data/rules.ron`.
- `scripts/ci.sh`: `cargo test --workspace` и `cargo build -p core --target wasm32-unknown-unknown`. Таргет wasm установить через `rustup target add`.

## Не делать
- Никакого состояния мира, событий, UI. Только фундамент.
- Не добавлять `rand`, `chrono`, любые крейты кроме `serde`, `ron`.

## Приёмка
- Тест: один seed даёт одну и ту же последовательность из 1000 `next_u64`, зафиксированную в тесте первыми пятью значениями.
- Тест: `range(0, 10)` на 10 000 вызовов никогда не выходит за границы, все 10 значений встречаются.
- Тест: перевод тиков в годы и обратно для `ticks_per_year` 1 и 4.
- Тест: `Fx` арифметика на граничных случаях (отрицательные, деление с остатком).
- Тест: `data/rules.ron` грузится.
- `scripts/ci.sh` проходит, включая wasm-сборку.

## Отчёт
Что сделано, список тестов, что отложено и почему, вопросы к дизайну.
