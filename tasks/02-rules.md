# Этап 2. События, эффекты, действия

## Контекст
Прочитай CLAUDE.md и DESIGN.md (разделы 4, 5.2 про силу короны, 7.1). Этапы 0 и 1 завершены.

## Вход
Ветка main после этапа 1. Типы `World`, `Axes`, `Province`, `Holder`, `Heir`, `Rng`, `Tick`, `Fx`. Поля `World` можно добавлять, существующие не менять.

## Сделать
Движок правил, который крутит правление без UI: действие → тик → событие → выбор.

- `Predicate` (RON, serde enum): `AxisAbove(AxisId, Fx)`, `AxisBelow`, `Flag(String)`, `NotFlag`, `ProvinceWhere(ProvinceFilter)` (есть хотя бы одна подходящая: по держателю, лояльности ниже/выше, постройке, соседству с иностранной), `HeirCount(u32, u32)`, `RulerAge(u32, u32)`, `AtWar`, `All(Vec)`, `Any(Vec)`, `Not(Box)`. `fn eval(&self, &World) -> bool`.
- `Effect` (RON): `Axis(AxisId, Fx)`, `Province(ProvinceTarget, ProvinceField, Fx)`, `SetFlag`, `ClearFlag`, `SpawnEvent(String, years: u32)`, `RulerHealth(Fx)`, `Relation(NeighbourId, Fx)`, `CrownPower(ProvinceTarget, Fx)`, `HeirOp(HeirOp)` с `Add | Remove(idx) | SetStatus(idx, HeirStatus) | Ability(idx, Fx)`. `ProvinceTarget = Capital | ById(ProvinceId) | EventTarget` (цель события, если оно привязано к провинции). `fn apply(&self, &mut World, ctx)`.
- `Event { id, title, text, when: Predicate, weight: u32, once: bool, cooldown_years: u32, importance: u32, target: EventTarget (None | RandomProvince(filter) | Neighbour), choices: Vec<Choice> }`, `Choice { text, effects: Vec<Effect>, cause_tag: String, hint: Option<String> }`. Текст может содержать `{province}`, `{neighbour}`, `{ruler}`, подставлять при показе.
- Планировщик: на тик собрать события, у которых `when` истинно и не на cooldown, плюс отложенные из очереди `(Tick, event_id)` с наступившим сроком. Отложенные приоритетнее. Взвешенный выбор одного по `Rng`. Не больше одного события за тик.
- `Action { id, name, duration_years, cost: Fx, requires: Predicate, min_crown_power: Fx, target: ActionTarget (None | Province(filter) | Neighbour | Heir), on_complete: Vec<Effect>, cause_tag }`. Слоты: `rules.ron` содержит `action_slots: [(bureaucracy_threshold, slots)]`.
- `Decision { tick, kind: EventChoice { event_id, choice_idx, target } | ActionStarted { action_id, target }, cause_tag }`, журнал `Game.decisions`.
- `Game { world, rng, data, decisions, pending_event: Option<PendingEvent>, queue }`. API: `new(data, preset, seed)`, `available_actions() -> Vec<(ActionId, Vec<Target>)>`, `start_action(id, target) -> Result<(), ActionError>`, `wait() -> Step` где `Step = Event(EventView) | Idle | ReignEnded(ReignEnd)` (пока `ReignEnded` не возвращается, это этап 5), `choose(idx) -> Result<(), _>`.
- Пассивный тик: казна += доход провинций короны минус содержание; старение правителя и наследников раз в год; лояльность фракций и провинций дрейфует к базовому значению на шаг из `rules.ron`; завершённые действия применяют `on_complete`; пересчёт силы короны.
- Тестовые данные `data/events/test.ron` (5 событий, одно с цепочкой через `SpawnEvent`) и `data/actions.ron` (2 действия). Этап 9 заменит содержимое, схему не трогает.

## Не делать
- Война, смерть, наследование, симуляция, UI.
- Скриптовый язык общего назначения. Предикаты и эффекты только перечисленные.

## Приёмка
- Тест: скрипт решений на 20 тиков с seed 42 даёт один и тот же `World` при повторе (сравнение целиком).
- Тест: действие с `min_crown_power` выше силы провинции отсутствует в `available_actions`.
- Тест: при одном слоте второе `start_action` возвращает ошибку; при двух проходит.
- Тест: `SpawnEvent(id, 2)` приводит к событию ровно через 2 года.
- Тест: событие с `once: true` не выпадает дважды.
- Тест: завершённое действие применяет `on_complete` в тик окончания.

## Отчёт
Что сделано, список тестов, что отложено и почему, вопросы к дизайну.
