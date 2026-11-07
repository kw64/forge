
(function(){
var D=window.SEARCH_INDEX||[];
var SYN={gpu:'nvenc qsv amf vaapi carte graphique acceleration',nvidia:'nvenc gpu',youtube:'en ligne telecharger yt-dlp url',telecharger:'en ligne yt-dlp',couper:'decoupe trim',decouper:'decoupe',joindre:'fusion',coller:'fusion concatener',logo:'incrustation watermark filigrane',filigrane:'logo incrustation','sous-titre':'sous-titres',soustitres:'sous-titres',mp3:'audio piste',couleur:'pipette',zip:'archive',rar:'archive',dossier:'repertoire',supprimer:'delete',doublon:'doublons',ia:'lm studio intelligence artificielle',film:'identifier tmdb',rom:'identifier dat',lent:'gpu vitesse preset',photo:'images',redimensionner:'images lot taille',trier:'tri classer',ranger:'tri classer',themes:'theme sombre clair',sombre:'theme dark',raccourcis:'clavier touche',aide:'premiers pas'};
function fold(s){return (s||'').toLowerCase().normalize('NFD').replace(/[\u0300-\u036f]/g,'');}
var base=(function(){var s=document.currentScript;return s&&s.dataset.base||'';})();
function terms(q){var t=fold(q).split(/[^a-z0-9+]+/).filter(Boolean),o=t.slice();t.forEach(function(w){if(SYN[w])SYN[w].split(' ').forEach(function(x){o.push(x)})});return {t:t,all:o};}
function search(q){var T=terms(q);if(!T.t.length)return[];var r=[];D.forEach(function(e){var sc=0,ft=fold(e.t),fk=fold(e.k),fb=fold(e.b);
 T.t.forEach(function(w){if(ft.indexOf(w)>=0)sc+=10;if(fk.indexOf(w)>=0)sc+=6;if(fb.indexOf(w)>=0)sc+=2;if(ft.indexOf(w)==0)sc+=3;});
 T.all.slice(T.t.length).forEach(function(w){if(fk.indexOf(w)>=0||ft.indexOf(w)>=0)sc+=2;});
 var miss=T.t.some(function(w){return ft.indexOf(w)<0&&fk.indexOf(w)<0&&fb.indexOf(w)<0&&!(SYN[w]&&SYN[w].split(' ').some(function(x){return fk.indexOf(x)>=0||ft.indexOf(x)>=0}))});
 if(sc>0&&!miss)r.push([sc,e]);});r.sort(function(a,b){return b[0]-a[0]});return r.slice(0,12).map(function(x){return x[1]});}
function hl(s,q){var T=terms(q).t;T.forEach(function(w){if(w.length<2)return;s=s.replace(new RegExp('('+w.replace(/[.*+?^${}()|[\]\\]/g,'\\$&')+')','ig'),'<mark>$1</mark>')});return s;}
function snip(e,q){var T=terms(q).t,b=e.b,i=-1;for(var k=0;k<T.length&&i<0;k++)i=fold(b).indexOf(T[k]);if(i<0)i=0;var s=Math.max(0,i-30);return (s?'…':'')+b.substr(s,110)+'…';}
var inp=document.getElementById('q'),box=document.getElementById('res'),sel=-1,cur=[];
if(!inp)return;
function render(){var q=inp.value.trim();cur=search(q);sel=cur.length?0:-1;if(!q){box.className='res';return;}
 box.innerHTML=cur.length?cur.map(function(e,i){return '<a href="'+base+e.u+'" class="'+(i==0?'sel':'')+'"><b class="app" style="color:'+e.c+'">'+e.a+'</b>'+hl(e.t,q)+'<small>'+hl(snip(e,q).replace(/</g,'&lt;'),q)+'</small></a>'}).join(''):'<a><b>Aucun résultat</b><small>Essayez un mot plus simple : « gpu », « logo », « renommer », « archive »…</small></a>';
 box.className='res on';}
function mv(d){var a=box.querySelectorAll('a');if(!a.length)return;a[sel]&&a[sel].classList.remove('sel');sel=(sel+d+a.length)%a.length;a[sel].classList.add('sel');a[sel].scrollIntoView({block:'nearest'});}
inp.addEventListener('input',render);inp.addEventListener('focus',render);
inp.addEventListener('keydown',function(ev){if(ev.key=='ArrowDown'){ev.preventDefault();mv(1)}else if(ev.key=='ArrowUp'){ev.preventDefault();mv(-1)}else if(ev.key=='Enter'&&cur[sel]){location.href=base+cur[sel].u}else if(ev.key=='Escape'){inp.value='';box.className='res';inp.blur()}});
document.addEventListener('click',function(e){if(!e.target.closest('.sw'))box.className='res';});
document.addEventListener('keydown',function(e){if((e.key=='/'&&!/INPUT|TEXTAREA/.test(document.activeElement.tagName))||((e.ctrlKey||e.metaKey)&&e.key.toLowerCase()=='k')){e.preventDefault();inp.focus();inp.select();}});
// thème
var th=document.getElementById('th');function ap(v){try{localStorage.setItem('fgTheme',v)}catch(e){}document.documentElement.setAttribute('data-theme',v)}
try{var s=localStorage.getItem('fgTheme');if(s)document.documentElement.setAttribute('data-theme',s)}catch(e){}
th&&th.addEventListener('click',function(){var d=document.documentElement.getAttribute('data-theme');var dark=d?d=='dark':matchMedia('(prefers-color-scheme:dark)').matches;ap(dark?'light':'dark')});
// sommaire actif
var links=[].slice.call(document.querySelectorAll('aside a[href^="#"]'));
if(links.length&&'IntersectionObserver' in window){var io=new IntersectionObserver(function(en){en.forEach(function(x){if(x.isIntersecting){links.forEach(function(l){l.classList.toggle('cur',l.getAttribute('href')=='#'+x.target.id)})}})},{rootMargin:'-80px 0px -70% 0px'});document.querySelectorAll('section.s').forEach(function(s){io.observe(s)});}
})();
