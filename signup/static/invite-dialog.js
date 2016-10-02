window.addEventListener("load", function() {
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
  function showInviteDialog() {
    var dialog = document.getElementById('invite_dialog');
    if (window.dialogPolyfill) {
      window.dialogPolyfill.registerDialog(dialog);
    }
    dialog.showModal();
  }
  document.getElementById('showInviteDialog').onclick = showInviteDialog;
  function closeInviteDialog() {
    var dialog = document.getElementById('invite_dialog');
    dialog.close();
  }
  document.getElementById('closeInviteDialog').onclick = closeInviteDialog;
});
