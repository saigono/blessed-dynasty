# Этап 1. Модель состояния

## Контекст
Прочитай CLAUDE.md и DESIGN.md (раздел 5 целиком, 4.1 про время). Этап 0 завершён.

## Вход
Ветка main после этапа 0. Типы: `Rng`, `Tick`, `TimeUnit`, `Fx`, `core::data::load`. Их сигнатуры не менять.

## Сделать
Модуль `core/src/state/` с типами без логики изменения, только данные, конструкторы и чтение.

- `AxisId(String)`, `Axes = BTreeMap<AxisId, Fx>`. Список осей, их начальные значения и границы приходят из `rules.ron` (`axes: [(id, min, max, default)]`). Стартовый список в данных: treasury, income, army, legitimacy, stability, bureaucracy, loyalty_nobles, loyalty_church, loyalty_people, prestige.
- `ProvinceId`, `Province { id, name, income: Fx, population: i32, loyalty: Fx, holder: Holder, buildings: BTreeSet<String>, neighbours: Vec<ProvinceId>, crown_power: Fx }`.
- `Holder = Crown | Vassal(VassalId) | Foreign(NeighbourId)`. Соседние государства состоят из таких же провинций.
- `Capital { province: ProvinceId, crown_bonus: Fx }`.
- `Faction { id: String, loyalty: Fx }`, список из данных. `Vassal { id, name, provinces: Vec<ProvinceId>, loyalty: Fx, strength: Fx }`.
- `Ruler { name, age: u32, health: Fx, traits: BTreeSet<String>, reign_start: Tick }`.
- `Heir { name, age, ability: Fx, claim: Fx, status: HeirStatus }`, `HeirStatus = Home | Hostage(NeighbourId) | Studying(String)`.
- `Neighbour { id, name, relation: Fx, strength: Fx, stance: Stance }`, `Stance = Expand | Defend | Trade | Wait`.
- `World { tick, time_unit, axes, provinces: BTreeMap<ProvinceId, Province>, capital, vassals, factions, ruler, heirs, neighbours, active_actions: Vec<ActiveAction>, flags: BTreeSet<String> }`. `ActiveAction` пока заглушка `{ id: String, target: Option<String>, ends_at: Tick }`.
- `derive(Serialize, Deserialize, Clone, Debug, PartialEq)` везде. `World::snapshot(&self) -> World` как `clone`, оставить как явный метод для хроники.
- Сила короны: `World::recompute_crown_power(&mut self, rules)`. Формула: база по держателю (Crown / Vassal / Foreign из `rules.ron`), минус `loyalty` ниже порога, минус расстояние от столицы в шагах × коэффициент, плюс бонус за постройки из списка, в столице плюс `crown_bonus`. Расстояние считать BFS по `neighbours`. Коэффициенты в `rules.ron`.
- Пресет `data/presets/default.ron` и загрузчик `World::from_preset(&Data, &Preset) -> World`. Пресет содержит оси, карту с полигонами для UI (пока пустые списки точек), вассалов, фракции, правителя, наследников, соседей. Карта в пресете минимальная: 3 провинции, 1 сосед, этап 3 заменит.

## Не делать
- Никаких событий, действий, тиков. Только структура данных и формула силы короны.
- Не делать трейты «Entity», generic-хранилища, ECS.

## Приёмка
- Тест: round-trip `World` через RON равен исходному.
- Тест: пресет грузится, число провинций и осей совпадает с данными.
- Тест: сила короны в столице выше, чем в провинции вассала на том же пресете.
- Тест: провинция с низкой лояльностью теряет силу короны относительно такой же с высокой.

## Отчёт
Что сделано, список тестов, что отложено и почему, вопросы к дизайну.
