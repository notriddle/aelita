if (!document.createElement('dialog').showModal) {
  var link = document.createElement('link');
  link.rel = 'stylesheet';
  link.href = 'static/dialog-polyfill.css';
  var script = document.createElement('script');
  script.src = 'static/dialog-polyfill.js';
  var head = document.getElementsByTagName('head')[0];
  head.appendChild(link);
  head.appendChild(script);
}
window.addEventListener("load", function() {
  function openDialog() {
    var dialog = document.getElementById(this.getAttribute('data-target'));
    if (window.dialogPolyfill) {
      window.dialogPolyfill.registerDialog(dialog);
    }
    dialog.showModal();
  }
  function closeDialog() {
    var dialog = document.getElementById(this.getAttribute('data-target'));
    dialog.close();
  }
  var ds = document.getElementsByClassName('openDialog');
  var dl = ds.length;
  for (i = 0; i != dl; ++i) {
    ds[i].onclick = openDialog;
  }
  var ds = document.getElementsByClassName('closeDialog');
  var dl = ds.length;
  for (i = 0; i != dl; ++i) {
    ds[i].onclick = closeDialog;
  }
});
