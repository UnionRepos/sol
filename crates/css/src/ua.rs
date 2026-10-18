//! User-agent default stylesheet — the HTML3/4 + common-HTML5 defaults
//! every real browser carries in its UA sheet.

pub const UA_CSS: &str = r#"
ol, ul { counter-reset: list-item; }
html, body, div, span, applet, object, iframe, h1, h2, h3, h4, h5, h6, p,
blockquote, pre, a, abbr, acronym, address, big, cite, code, del, dfn, em,
img, ins, kbd, q, s, samp, small, strike, strong, sub, sup, tt, var, b, u,
i, center, dl, dt, dd, ol, ul, li, fieldset, form, label, legend, table,
caption, tbody, tfoot, thead, tr, th, td, article, aside, canvas, details,
embed, figure, figcaption, footer, header, hgroup, menu, nav, output, ruby,
section, summary, time, mark, audio, video, main, dialog {
    margin: 0; padding: 0; border: 0;
}
html { display: block; }
body { display: block; margin: 8px; }
head, title, meta, link, style, script, base, noscript { display: none; }
div, p, dl, dt, dd, ol, ul, li, article, aside, section, nav, header,
footer, main, figure, figcaption, fieldset, form, legend, dialog,
blockquote, pre, address, hgroup { display: block; }
h1 { display: block; font-size: 2em; font-weight: bold; margin: 0.67em 0; }
h2 { display: block; font-size: 1.5em; font-weight: bold; margin: 0.83em 0; }
h3 { display: block; font-size: 1.17em; font-weight: bold; margin: 1em 0; }
h4 { display: block; font-size: 1em; font-weight: bold; margin: 1.33em 0; }
h5 { display: block; font-size: 0.83em; font-weight: bold; margin: 1.67em 0; }
h6 { display: block; font-size: 0.67em; font-weight: bold; margin: 2.33em 0; }
p { display: block; margin: 1em 0; }
pre { display: block; font-family: monospace; white-space: pre; margin: 1em 0; }
blockquote { display: block; margin: 1em 40px; }
ul, menu, dir { display: block; list-style-type: disc; margin: 1em 0; padding-left: 40px; }
ol { counter-reset: list-item; display: block; list-style-type: decimal; margin: 1em 0; padding-left: 40px; }
li { display: list-item; }
ul ul, ol ul { counter-reset: list-item; list-style-type: circle; }
ul ul ul, ul ol ul, ol ul ul, ol ol ul { list-style-type: square; }
a { color: #0000ee; text-decoration: underline; cursor: pointer; }
a:visited { color: #551a8b; }
b, strong { font-weight: bold; }
i, em, cite, var, address, dfn { font-style: italic; }
u, ins { text-decoration: underline; }
s, strike, del { text-decoration: line-through; }
code, kbd, samp, tt { font-family: monospace; }
pre, xmp, plaintext, listing { font-family: monospace; white-space: pre; }
big { font-size: larger; }
small { font-size: smaller; }
sub { vertical-align: sub; font-size: smaller; }
sup { vertical-align: super; font-size: smaller; }
center { display: block; text-align: center; }
hr { display: block; margin: 0.5em auto; border-width: 1px; border-style: inset; }
table { display: table; border-collapse: separate; border-spacing: 2px; }
thead { display: table-header-group; vertical-align: middle; }
tbody { display: table-row-group; vertical-align: middle; }
tfoot { display: table-footer-group; vertical-align: middle; }
tr { display: table-row; vertical-align: inherit; }
td, th { display: table-cell; vertical-align: inherit; padding: 1px; }
th { font-weight: bold; text-align: center; }
caption { display: table-caption; text-align: center; }
colgroup { display: table-column-group; }
col { display: table-column; }
img { display: inline-block; }
br { display: inline; }
br[clear] { clear: both; }
span { display: inline; }
nobr { white-space: nowrap; }
mark { background: yellow; color: black; }
abbr[title], acronym[title] { text-decoration: dotted underline; }
input, button, select, textarea { display: inline-block; font-family: inherit; font-size: inherit; }
select { display: inline-block; }
option, optgroup { display: block; }
q::before { content: open-quote; }
q::after { content: close-quote; }
[hidden] { display: none; }
"#;
