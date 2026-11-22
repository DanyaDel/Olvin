// Adds a language switch (ru <-> en) to the menu bar of every page.
(function () {
  var path = location.pathname;
  var m = path.match(/^(.*\/)(ru|en)(\/.*)?$/);
  if (!m) return;
  var from = m[2], to = from === "ru" ? "en" : "ru";
  var target = m[1] + to + (m[3] || "/");
  var a = document.createElement("a");
  a.href = target + location.hash;
  a.className = "lang-switch";
  a.title = to === "en" ? "English version of this page" : "Русская версия этой страницы";
  a.textContent = to === "en" ? "English" : "Русский";
  var bar = document.querySelector(".right-buttons");
  if (bar) bar.insertBefore(a, bar.firstChild);
})();
