# Этап 4. Война

## Контекст
Прочитай CLAUDE.md и DESIGN.md (раздел 6). Этапы 0–3 завершены.

## Вход
Ветка main после этапа 3. Типы `World`, `Game`, `Event`, `Effect`, `Neighbour`, `Holder::Foreign`. Сигнатуры не менять. Разрешено добавить варианты в `Effect` и `Predicate`.

## Сделать
- `War { enemy: NeighbourId, stage: WarStage, our_strength: Fx, their_strength: Fx, war_score: Fx, started: Tick }` в `World.war: Option<War>`. Одна война за раз.
- Формула столкновения: `our = army × (1 + fort_bonus + nobles_bonus) × treasury_factor`, аналогично у соседа по `strength`; бросок `Rng` в диапазоне из `rules.ron`; `war_score += (our − their) × k`. Всё в `Fx`.
- Новые эффекты: `StartWar(NeighbourId)`, `Clash`, `TransferProvince(ProvinceTarget, Holder)`, `Tribute(Fx)`, `TakeHostage(heir_idx, NeighbourId)`, `EndWar(WarOutcome)`. Предикаты: `WarScoreAbove(Fx)`, `WarScoreBelow`, `WarStage(WarStage)`.
- Цепочка в `data/events/war.ron`: `war_declared` (выбор: наёмники за казну, призыв вассалов за лояльность, малое войско) → `war_clash` (Clash, выбор: наступать, держать, отступить) → `war_siege_or_talks` (по `war_score`: осада пограничной провинции или переговоры) → `war_outcome` (победа: `TransferProvince` + `Tribute`; поражение: потеря провинции, `Tribute`, `TakeHostage`, `RulerHealth` вниз, легитимность вниз; ничья). Каждый шаг спавнит следующий через `SpawnEvent`.
- Действие `declare_war` (цель: сосед) и событие `neighbour_war_declared` от ИИ ведут в `war_declared`.
- Провинции при `TransferProvince` меняют `holder` целиком; при переходе к короне получают `crown_power` по формуле.

## Не делать
- Тактика, несколько фронтов, союзники.
- Симуляция и UI.

## Приёмка
- Тест: при силе 3:1 в нашу пользу война выигрывается в ≥ 90% из 1000 прогонов по разным seed; при 1:3 проигрывается в ≥ 90%.
- Тест: цепочка всегда завершается `EndWar` не позже чем через 6 лет после объявления.
- Тест: после победы провинция противника имеет `Holder::Crown`, после поражения пограничная провинция короны имеет `Holder::Foreign`.
- Тест: вторая `StartWar` во время войны отклоняется.

## Отчёт
Что сделано, список тестов, что отложено и почему, вопросы к дизайну.
