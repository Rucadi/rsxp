// SPDX-License-Identifier: GPL-2.0-or-later
import { createTouchInput } from './input.js';
const $ = id => document.getElementById(id);
const decode = value => Uint8Array.from(atob(value), c => c.charCodeAt(0));
function native(request) {
  const raw = prompt('rsxp:' + JSON.stringify(request), '');
  if (!raw) throw new Error('Native Rust bridge is unavailable. Open this interface in the iOS or Android app.');
  const response = JSON.parse(raw);
  if (response.error) throw new Error(response.error);
  return response.ok;
}
let running = false, paused = false, generation = 0;
let logs = [];
let nativePresentation=false;
const touch = createTouchInput((code, down) => { if (running) native({ op: 'key', code, down }); });
function releaseControls() {
  touch.releaseAll();
  document.querySelectorAll('.held').forEach(button => button.classList.remove('held'));
}
function status(message) { $('status').textContent = message; $('status').hidden = !message; }
function failure(error) {
  console.error(error);
  stop();
  $('error-message').textContent = String(error.message || error) + (logs.length ? '\n\n' + logs.slice(-20).join('\n') : '');
  $('error').showModal();
}
function stop() {
  releaseControls();
  running = false; paused = false; generation++;
  $('pause').textContent = 'Pause';
  try { native({ op: 'stop' }); } catch { /* Initial bridge errors are reported separately. */ }
  $('player').hidden = true; $('library').hidden = false; status('');
}
function refresh() {
  const games = native({ op: 'games' });
  $('games').replaceChildren(...games.map(game => {
    const button = document.createElement('button'); button.className = 'game';
    const icon = document.createElement('span'); icon.className = 'game-icon'; icon.textContent = game.demo ? '✦' : '▧';
    const text = document.createElement('span');
    const title = document.createElement('strong'); title.textContent = game.title;
    const detail = document.createElement('small'); detail.textContent = game.demo ? 'Built in · Test the native engine' : 'Imported game · Tap to start';
    text.append(title, detail);
    const arrow = document.createElement('span'); arrow.className = 'arrow'; arrow.textContent = '›';
    button.append(icon, text, arrow); button.onclick = () => start(game).catch(failure); return button;
  }));
}
const nextFrame = () => new Promise(resolve => requestAnimationFrame(resolve));
async function start(game) {
  if (running) return;
  status('Starting native Rust engine…'); logs = [];
  await nextFrame();
  const config = native({ op: 'init', id: game.id });
  status('Starting native Ruby scripts…'); await nextFrame();
  running = true; paused = false; const current = ++generation;
  status('');
  $('title').textContent = config.title; $('library').hidden = true; $('player').hidden = false;
  const context = nativePresentation ? null : $('screen').getContext('2d', {alpha:false});
  if(nativePresentation) positionFrame();
  let deadline = 0;
  function pump(now) {
    if (!running || current !== generation) return;
    if (!paused && now >= deadline) {
      if (!deadline) deadline = now;
      // Preserve the cadence: adding a period to `now` caps 40 fps games at
      // 30 fps on a 60 Hz display. Discard backlog after a pause or slow frame.
      deadline = Math.max(deadline + 1000 / config.rate, now);
      try {
        if (!native({op:'step'}).running) { stop(); return; }
        if (nativePresentation) { requestAnimationFrame(pump); return; }
        const frame = native({op:'frame'});
        if ($('screen').width !== frame.width || $('screen').height !== frame.height) {
          $('screen').width = frame.width; $('screen').height = frame.height;
        }
        context.putImageData(new ImageData(new Uint8ClampedArray(decode(frame.rgba).buffer),frame.width,frame.height),0,0);
      } catch (error) { failure(error); return; }
    }
    requestAnimationFrame(pump);
  }
  requestAnimationFrame(pump);
}
for (const button of document.querySelectorAll('[data-key]')) {
  button.onpointerdown = event => {
    event.preventDefault(); if (!running || paused) return;
    button.setPointerCapture(event.pointerId); button.classList.add('held');
    touch.press(event.pointerId, Number(button.dataset.key));
  };
  for (const type of ['pointerup','pointercancel','lostpointercapture']) button.addEventListener(type,event => {
    touch.release(event.pointerId); button.classList.remove('held');
  });
}
document.addEventListener('visibilitychange', () => { if (document.hidden && running) { releaseControls(); paused = true; $('pause').textContent = 'Resume'; } });
window.addEventListener('blur', releaseControls);
const keys = {ArrowUp:82,ArrowDown:81,ArrowLeft:80,ArrowRight:79,KeyZ:29,Enter:40,KeyX:27,Escape:41,ShiftLeft:225};
window.addEventListener('keydown',event => { if (keys[event.code] && running && !paused) { event.preventDefault(); touch.press(event.code,keys[event.code]); } });
window.addEventListener('keyup',event => touch.release(event.code));
$('pause').onclick = () => { releaseControls(); paused = !paused; $('pause').textContent = paused ? 'Resume' : 'Pause'; };
$('back').onclick = stop;
$('dismiss').onclick = () => { $('error').close(); refresh(); };
$('refresh').onclick = () => { try { refresh(); } catch(error) { failure(error); } };
$('import').onclick = () => { try { status('Choose the game folder in Files…'); native({op:'import'}); } catch(error) { failure(error); } };
globalThis.RSXPMobileImported = error => { status(''); if (error) failure(new Error(error)); else refresh(); };
try {
  const info = native({op:'info'}); nativePresentation=info.nativePresentation;
  $('platform').textContent = `${info.engine} · ${info.platform} / ${info.arch}`; refresh();
  const parameters = new URL(location.href).searchParams;
  if (parameters.has('game')) {
    start({id:parameters.get('game')}).catch(failure);
  } else if (parameters.get('demo') === '1') {
    start({id:'.demo'}).catch(failure);
  }
}
catch(error) { failure(error); }

function positionFrame() {
  if(!nativePresentation) return;
  const r=$('viewport').getBoundingClientRect();
  native({op:'viewport',x:r.x,y:r.y,width:r.width,height:r.height});
}
window.addEventListener('resize',()=>{if(running) requestAnimationFrame(positionFrame);});
