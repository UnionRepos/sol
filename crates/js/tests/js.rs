use sol_dom::Document;
use sol_js::{Js, V};
use std::cell::RefCell;
use std::rc::Rc;

fn run(src: &str) -> V {
    let mut js = Js::new();
    ev(&mut js, src)
}

fn ev(js: &mut Js, src: &str) -> V {
    match js.eval(src) {
        Ok(v) => v,
        Err(t) => panic!(
            "eval failed: {} for {}",
            t.fatal.clone().unwrap_or_else(|| t.v.to_str(&js.rt)),
            src
        ),
    }
}

fn num(v: &V) -> f64 {
    match v {
        V::Num(n) => *n,
        _ => panic!("not num: {:?}", v),
    }
}

#[test]
fn arithmetic_and_vars() {
    assert_eq!(num(&run("1 + 2 * 3")), 7.0);
    assert_eq!(num(&run("let x = 5; x * x - 1")), 24.0);
    assert_eq!(num(&run("var a = 10; a += 5; a /= 3; a")), 5.0);
    assert_eq!(
        num(&run("let s = 0; for (let i = 0; i < 10; i++) s += i; s")),
        45.0
    );
    assert_eq!(
        num(&run(
            "let i = 3; let t = 0; while (i > 0) { t += i; i--; } t"
        )),
        6.0
    );
    assert_eq!(
        num(&run("let s = 0; for (const x of [1,2,3]) s += x; s")),
        6.0
    );
    assert_eq!(
        num(&run("let s = 0; for (const k in {a:1,b:2}) s++; s")),
        2.0
    );
}

#[test]
fn functions_and_closures() {
    assert_eq!(
        num(&run("function add(a, b) { return a + b; } add(3, 4)")),
        7.0
    );
    assert_eq!(num(&run("let mk = (x) => (y) => x + y; mk(10)(5)")), 15.0);
    assert_eq!(
        num(&run(
            "let mkc = () => { let n = 0; return () => { n++; return n; }; }; \
             let c = mkc(); c(); c(); c()"
        )),
        3.0
    );
    assert_eq!(
        num(&run(
            "let fib = (n) => n < 2 ? n : fib(n-1) + fib(n-2); fib(10)"
        )),
        55.0
    );
    // call/apply/bind
    assert_eq!(
        num(&run(
            "let f = function(a) { return this.n + a; }; f.call({n: 3}, 4)"
        )),
        7.0
    );
    assert_eq!(
        num(&run(
            "let f = function(a, b) { return a + b; }; f.apply(null, [3, 4])"
        )),
        7.0
    );
    assert_eq!(
        num(&run(
            "let f = function() { return this.n; }; f.bind({n: 9})()"
        )),
        9.0
    );
    // rest params + spread call
    assert_eq!(
        num(&run("let f = (a, ...r) => r.length + a; f(1, 2, 3)")),
        3.0
    );
    assert_eq!(num(&run("let f = (a,b,c) => a+b+c; f(...[1,2,3])")), 6.0);
}

#[test]
fn arrow_this() {
    assert_eq!(
        num(&run(
            "let o = { n: 7, get: function() { let f = () => this.n; return f(); } }; \
             let g = o.get; g.call(o)"
        )),
        7.0
    );
    assert_eq!(
        num(&run(
            "let o = { n: 9, m: function() { let a = () => this.n + 1; return a(); } }; o.m()"
        )),
        10.0
    );
}

#[test]
fn arrays() {
    assert_eq!(num(&run("let a = [1,2,3]; a.length")), 3.0);
    assert_eq!(
        num(&run(
            "let a = [1,2,3,4]; a.map((x) => x * 2).reduce((s, x) => s + x, 0)"
        )),
        20.0
    );
    assert_eq!(num(&run("[5,1,4,2].filter((x) => x > 2).length")), 2.0);
    assert_eq!(num(&run("[3,1,2].sort().indexOf(1)")), 0.0);
    match run("[1,2,3].join('-')") {
        V::Str(s) => assert_eq!(&*s, "1-2-3"),
        v => panic!("expected string, got {:?}", v),
    }
    assert_eq!(
        num(&run(
            "let a = [1,2]; a.push(3,4); let b = a.slice(1,3); b[0] + a.length + a.pop()"
        )),
        2.0 + 4.0 + 4.0
    );
    assert_eq!(
        num(&run("let a = [1,2,3]; a.splice(1,1); a.length + a[1]")),
        5.0
    );
    assert_eq!(
        num(&run("let s = 0; [1,2,3].forEach((x) => { s += x; }); s")),
        6.0
    );
}

#[test]
fn strings_and_regex() {
    match run("'hello'.toUpperCase()") {
        V::Str(s) => assert_eq!(&*s, "HELLO"),
        v => panic!("bad: {:?}", v),
    }
    assert_eq!(num(&run("'a,b,c'.split(',').length")), 3.0);
    match run("'hello world'.replace(/o/, '0')") {
        V::Str(s) => assert_eq!(&*s, "hell0 world"),
        v => panic!("bad: {:?}", v),
    }
    match run("/\\d+/.test('abc123')") {
        V::Bool(b) => assert!(b),
        v => panic!("bad: {:?}", v),
    }
    match run("'2024-01-15'.match(/(\\d+)/)[1]") {
        V::Str(s) => assert_eq!(&*s, "2024"),
        v => panic!("bad: {:?}", v),
    }
    match run("let n = 'w'; `hi ${n}!`") {
        V::Str(s) => assert_eq!(&*s, "hi w!"),
        v => panic!("bad: {:?}", v),
    }
    assert_eq!(num(&run("'abc'.charCodeAt(1)")), 98.0);
    assert_eq!(num(&run("' x '.trim().length")), 1.0);
}

#[test]
fn objects_and_classes() {
    assert_eq!(num(&run("let o = {a: 1, b: 2}; o.a + o.b")), 3.0);
    assert_eq!(
        num(&run(
            "class P { constructor(x) { this.x = x; } get() { return this.x; } } \
             let p = new P(8); p.get()"
        )),
        8.0
    );
    assert_eq!(
        num(&run("class A { constructor() { this.v = 1; } } \
             class B extends A { constructor() { super(); this.v += 5; } } \
             new B().v")),
        6.0
    );
    assert_eq!(
        num(&run(
            "let o = {}; Object.keys({a:1,b:2}).length + Object.keys(o).length"
        )),
        2.0
    );
    match run("JSON.stringify({a: 1})") {
        V::Str(s) => assert_eq!(&*s, r#"{"a":1}"#),
        v => panic!("bad: {:?}", v),
    }
    assert_eq!(num(&run("JSON.parse('{\"x\": 5}').x")), 5.0);
    assert_eq!(num(&run("Math.max(3, 9, 2) + Math.floor(1.9)")), 10.0);
}

#[test]
fn promises_and_microtasks() {
    let mut js = Js::new();
    ev(
        &mut js,
        "let order = []; Promise.resolve(1).then((v) => order.push(v)); order.push(0);",
    );
    match ev(&mut js, "order.join(',')") {
        V::Str(s) => assert_eq!(&*s, "0,1"),
        v => panic!("bad: {:?}", v),
    }
    // chained then resolves through microtask queue; capture via global
    ev(
        &mut js,
        "let pv; Promise.resolve(2).then((x) => x * 3).then((x) => { pv = x + 1; });",
    );
    assert_eq!(num(&ev(&mut js, "pv")), 7.0);
    ev(
        &mut js,
        "let cv; Promise.reject('e').catch((x) => { cv = 'caught:' + x; });",
    );
    match ev(&mut js, "cv") {
        V::Str(s) => assert_eq!(&*s, "caught:e"),
        v => panic!("bad: {:?}", v),
    }
    ev(&mut js, "let av; Promise.all([Promise.resolve(1), Promise.resolve(2)]).then((a) => { av = a[0] + a[1]; });");
    assert_eq!(num(&ev(&mut js, "av")), 3.0);
    ev(
        &mut js,
        "let rv; Promise.race([Promise.resolve(4), Promise.resolve(5)]).then((x) => { rv = x; });",
    );
    assert_eq!(num(&ev(&mut js, "rv")), 4.0);
}

#[test]
fn map_set() {
    assert_eq!(
        num(&run(
            "let m = new Map(); m.set('a', 1); m.set('b', 2); m.get('b') + m.size"
        )),
        4.0
    );
    assert_eq!(num(&run("let m = new Map([['x', 9]]); m.get('x')")), 9.0);
    match run("new Map([['x', 9]]).has('x')") {
        V::Bool(b) => assert!(b),
        v => panic!("bad: {:?}", v),
    }
    assert_eq!(
        num(&run(
            "let s = new Set(); s.add(1); s.add(1); s.add(2); s.size"
        )),
        2.0
    );
    assert_eq!(
        num(&run(
            "let s = 0; new Set([1,2,3]).forEach((x) => { s += x; }); s"
        )),
        6.0
    );
}

#[test]
fn error_handling() {
    assert_eq!(
        num(&run("let r = 0; try { throw 42; } catch (e) { r = e; } r")),
        42.0
    );
    assert_eq!(
        num(&run("let f = () => { try { return 1; } finally { } }; f()")),
        1.0
    );
    match run("try { let x = notdef123; } catch (e) { 'caught' }") {
        V::Str(s) => assert_eq!(&*s, "caught"),
        v => panic!("bad: {:?}", v),
    }
    // instanceof via Error subtype
    match run("let e = new TypeError('x'); e instanceof TypeError") {
        V::Bool(b) => assert!(b),
        v => panic!("bad: {:?}", v),
    }
}

#[test]
fn dom_basics() {
    let mut js = Js::new();
    let doc = Rc::new(RefCell::new(Document::new()));
    {
        let mut d = doc.borrow_mut();
        sol_html::parse(
            "<html><body><div id='app'><p class='x'>hi</p></div></body></html>",
            &mut d,
        );
    }
    js.set_document(doc.clone());
    match ev(&mut js, "document.querySelector('.x').textContent") {
        V::Str(s) => assert_eq!(&*s, "hi"),
        v => panic!("bad: {:?}", v),
    }
    match ev(
        &mut js,
        "let app = document.getElementById('app'); \
         app.classList.add('ready'); app.classList.contains('ready')",
    ) {
        V::Bool(true) => {}
        v => panic!("bad: {:?}", v),
    }
    assert_eq!(
        num(&ev(
            &mut js,
            "let d2 = document.createElement('span'); \
             d2.textContent = 'new'; \
             document.getElementById('app').appendChild(d2); \
             document.querySelectorAll('span').length"
        )),
        1.0
    );
    match ev(
        &mut js,
        "let e = document.querySelector('p'); \
         e.setAttribute('data-k', 'v1'); \
         e.style.color = 'red'; \
         e.getAttribute('data-k') + '|' + e.style.color",
    ) {
        V::Str(s) => assert_eq!(&*s, "v1|red"),
        v => panic!("bad: {:?}", v),
    }
    assert_eq!(
        doc.borrow()
            .get_element_by_id("app")
            .map(|n| doc.borrow().attr(n, "class").unwrap_or("").to_string())
            .unwrap_or_default(),
        "ready"
    );
}

#[test]
fn insert_adjacent_html_preserves_position_and_fragment_order() {
    for (position, parent_text, target_text) in [
        ("beforebegin", "firstsecondold", "old"),
        ("afterbegin", "firstsecondold", "firstsecondold"),
        ("beforeend", "oldfirstsecond", "oldfirstsecond"),
        ("afterend", "oldfirstsecond", "old"),
        ("", "oldfirstsecond", "oldfirstsecond"),
        ("invalid", "old", "old"),
    ] {
        let mut doc = Document::new();
        sol_html::parse(
            "<body><section id='parent'><div id='target'><b>old</b></div></section>",
            &mut doc,
        );
        let doc = Rc::new(RefCell::new(doc));
        let mut js = Js::new();
        js.set_document(doc.clone());
        ev(
            &mut js,
            &format!(
                "document.getElementById('target').insertAdjacentHTML(\
                 '{position}', '<i>first</i><em>second</em>');"
            ),
        );
        let doc = doc.borrow();
        let parent = doc.get_element_by_id("parent").unwrap();
        let target = doc.get_element_by_id("target").unwrap();
        assert_eq!(doc.text_content(parent), parent_text, "{position:?}");
        assert_eq!(doc.text_content(target), target_text, "{position:?}");
    }
}

#[test]
fn dom_events() {
    let mut js = Js::new();
    let doc = Rc::new(RefCell::new(Document::new()));
    {
        let mut d = doc.borrow_mut();
        sol_html::parse(
            "<html><body><button id='b'>go</button></body></html>",
            &mut d,
        );
    }
    js.set_document(doc.clone());
    ev(
        &mut js,
        "globalThis.clicks = 0; \
         document.getElementById('b').addEventListener('click', (ev) => { \
             clicks++; globalThis.lastType = ev.type; \
         });",
    );
    let btn = doc.borrow().get_element_by_id("b").unwrap();
    js.fire_event(btn, "click");
    assert_eq!(num(&ev(&mut js, "clicks")), 1.0);
    match ev(&mut js, "lastType") {
        V::Str(s) => assert_eq!(&*s, "click"),
        v => panic!("bad: {:?}", v),
    }
}

#[test]
fn gc_survives() {
    let mut js = Js::new();
    ev(
        &mut js,
        "let keep = [1,2,3]; for (let i = 0; i < 1000; i++) { let tmp = {x: i}; }",
    );
    js.gc();
    assert_eq!(num(&ev(&mut js, "keep.reduce((s, x) => s + x, 0)")), 6.0);
}
