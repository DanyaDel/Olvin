# Консоль и профилировщик

Два элемента редактора для отладки: **Console** (ставится по умолчанию) и **Profiler** (`olvin-hub editor add profiler`).

## Console

Панель лога редактора: фильтры по уровню и тексту, **Copy** для показанных строк и **Copy line** для одной. Одна запись это одна строка экрана; длинные и многострочные сообщения обрезаются, полный текст в подсказке. В памяти до 5000 строк. Лог редактора также пишется в `<проект>/logs/editor.log`.

## Profiler

Панель «⏱ Profiler» (**Window → Add Panel**):

- график времени кадра (линии 60 и 30 fps) и кнопка **Pause**;
- счётчики рендера: Draw calls, Instances, Culled, Triangles (сумма по видам редактора);
- таймлайн scopes кадра (полосы по глубине);
- таблица **Systems and passes**: имя, мс, вызовы, из кадров puffin.

Запись scopes включается, пока панель открыта. В `EditorApp::frame` свой scope `editor frame`. Сами scopes (системы, стадии, рендер, физика, скрипты) описаны в [Профилировщик](../engine/profiler.md).

Оверлей статистики в углу вьюпорта: Settings → Rendering → Editor views.

## См. также

- [Профилировщик (движок)](../engine/profiler.md)
- [Медленные кадры](../troubleshooting/performance.md)
- [Сцена и вьюпорт](scene-view.md)
- [Настройки](settings.md)
