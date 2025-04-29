import { HTMLNode } from './html-node.class.ts';


export function A(debugName?: string): HTMLNode<HTMLAnchorElement> {
	const element = document.createElement('a') as HTMLAnchorElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Abbr(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('abbr') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Address(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('address') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Area(debugName?: string): HTMLNode<HTMLAreaElement> {
	const element = document.createElement('area') as HTMLAreaElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Article(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('article') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Aside(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('aside') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Audio(debugName?: string): HTMLNode<HTMLAudioElement> {
	const element = document.createElement('audio') as HTMLAudioElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function B(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('b') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Base(debugName?: string): HTMLNode<HTMLBaseElement> {
	const element = document.createElement('base') as HTMLBaseElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Bdi(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('bdi') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Bdo(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('bdo') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Blockquote(debugName?: string): HTMLNode<HTMLQuoteElement> {
	const element = document.createElement('blockquote') as HTMLQuoteElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Body(debugName?: string): HTMLNode<HTMLBodyElement> {
	const element = document.createElement('body') as HTMLBodyElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Br(debugName?: string): HTMLNode<HTMLBRElement> {
	const element = document.createElement('br') as HTMLBRElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Button(debugName?: string): HTMLNode<HTMLButtonElement> {
	const element = document.createElement('button') as HTMLButtonElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Canvas(debugName?: string): HTMLNode<HTMLCanvasElement> {
	const element = document.createElement('canvas') as HTMLCanvasElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Caption(debugName?: string): HTMLNode<HTMLTableCaptionElement> {
	const element = document.createElement('caption') as HTMLTableCaptionElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Cite(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('cite') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Code(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('code') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Col(debugName?: string): HTMLNode<HTMLTableColElement> {
	const element = document.createElement('col') as HTMLTableColElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Colgroup(debugName?: string): HTMLNode<HTMLTableColElement> {
	const element = document.createElement('colgroup') as HTMLTableColElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Data(debugName?: string): HTMLNode<HTMLDataElement> {
	const element = document.createElement('data') as HTMLDataElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Datalist(debugName?: string): HTMLNode<HTMLDataListElement> {
	const element = document.createElement('datalist') as HTMLDataListElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Dd(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('dd') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Del(debugName?: string): HTMLNode<HTMLModElement> {
	const element = document.createElement('del') as HTMLModElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Details(debugName?: string): HTMLNode<HTMLDetailsElement> {
	const element = document.createElement('details') as HTMLDetailsElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Dfn(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('dfn') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Dialog(debugName?: string): HTMLNode<HTMLDialogElement> {
	const element = document.createElement('dialog') as HTMLDialogElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Div(debugName?: string): HTMLNode<HTMLDivElement> {
	const element = document.createElement('div') as HTMLDivElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Dl(debugName?: string): HTMLNode<HTMLDListElement> {
	const element = document.createElement('dl') as HTMLDListElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Dt(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('dt') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Em(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('em') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Embed(debugName?: string): HTMLNode<HTMLEmbedElement> {
	const element = document.createElement('embed') as HTMLEmbedElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Fieldset(debugName?: string): HTMLNode<HTMLFieldSetElement> {
	const element = document.createElement('fieldset') as HTMLFieldSetElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Figcaption(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('figcaption') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Figure(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('figure') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Footer(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('footer') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Form(debugName?: string): HTMLNode<HTMLFormElement> {
	const element = document.createElement('form') as HTMLFormElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function H1(debugName?: string): HTMLNode<HTMLHeadingElement> {
	const element = document.createElement('h1') as HTMLHeadingElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function H2(debugName?: string): HTMLNode<HTMLHeadingElement> {
	const element = document.createElement('h2') as HTMLHeadingElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function H3(debugName?: string): HTMLNode<HTMLHeadingElement> {
	const element = document.createElement('h3') as HTMLHeadingElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function H4(debugName?: string): HTMLNode<HTMLHeadingElement> {
	const element = document.createElement('h4') as HTMLHeadingElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function H5(debugName?: string): HTMLNode<HTMLHeadingElement> {
	const element = document.createElement('h5') as HTMLHeadingElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function H6(debugName?: string): HTMLNode<HTMLHeadingElement> {
	const element = document.createElement('h6') as HTMLHeadingElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Head(debugName?: string): HTMLNode<HTMLHeadElement> {
	const element = document.createElement('head') as HTMLHeadElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Header(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('header') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Hgroup(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('hgroup') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Hr(debugName?: string): HTMLNode<HTMLHRElement> {
	const element = document.createElement('hr') as HTMLHRElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Html(debugName?: string): HTMLNode<HTMLHtmlElement> {
	const element = document.createElement('html') as HTMLHtmlElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function I(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('i') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Iframe(debugName?: string): HTMLNode<HTMLIFrameElement> {
	const element = document.createElement('iframe') as HTMLIFrameElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Img(debugName?: string): HTMLNode<HTMLImageElement> {
	const element = document.createElement('img') as HTMLImageElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Input(debugName?: string): HTMLNode<HTMLInputElement> {
	const element = document.createElement('input') as HTMLInputElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Ins(debugName?: string): HTMLNode<HTMLModElement> {
	const element = document.createElement('ins') as HTMLModElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Kbd(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('kbd') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Label(debugName?: string): HTMLNode<HTMLLabelElement> {
	const element = document.createElement('label') as HTMLLabelElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Legend(debugName?: string): HTMLNode<HTMLLegendElement> {
	const element = document.createElement('legend') as HTMLLegendElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Li(debugName?: string): HTMLNode<HTMLLIElement> {
	const element = document.createElement('li') as HTMLLIElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Link(debugName?: string): HTMLNode<HTMLLinkElement> {
	const element = document.createElement('link') as HTMLLinkElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Main(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('main') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Map(debugName?: string): HTMLNode<HTMLMapElement> {
	const element = document.createElement('map') as HTMLMapElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Mark(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('mark') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Menu(debugName?: string): HTMLNode<HTMLMenuElement> {
	const element = document.createElement('menu') as HTMLMenuElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Meta(debugName?: string): HTMLNode<HTMLMetaElement> {
	const element = document.createElement('meta') as HTMLMetaElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Meter(debugName?: string): HTMLNode<HTMLMeterElement> {
	const element = document.createElement('meter') as HTMLMeterElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Nav(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('nav') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Noscript(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('noscript') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Object(debugName?: string): HTMLNode<HTMLObjectElement> {
	const element = document.createElement('object') as HTMLObjectElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Ol(debugName?: string): HTMLNode<HTMLOListElement> {
	const element = document.createElement('ol') as HTMLOListElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Optgroup(debugName?: string): HTMLNode<HTMLOptGroupElement> {
	const element = document.createElement('optgroup') as HTMLOptGroupElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Option(debugName?: string): HTMLNode<HTMLOptionElement> {
	const element = document.createElement('option') as HTMLOptionElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Output(debugName?: string): HTMLNode<HTMLOutputElement> {
	const element = document.createElement('output') as HTMLOutputElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function P(debugName?: string): HTMLNode<HTMLParagraphElement> {
	const element = document.createElement('p') as HTMLParagraphElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Param(debugName?: string): HTMLNode<HTMLParamElement> {
	const element = document.createElement('param') as HTMLParamElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Picture(debugName?: string): HTMLNode<HTMLPictureElement> {
	const element = document.createElement('picture') as HTMLPictureElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Pre(debugName?: string): HTMLNode<HTMLPreElement> {
	const element = document.createElement('pre') as HTMLPreElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Progress(debugName?: string): HTMLNode<HTMLProgressElement> {
	const element = document.createElement('progress') as HTMLProgressElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Q(debugName?: string): HTMLNode<HTMLQuoteElement> {
	const element = document.createElement('q') as HTMLQuoteElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Rp(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('rp') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Rt(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('rt') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Ruby(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('ruby') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function S(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('s') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Samp(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('samp') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Script(debugName?: string): HTMLNode<HTMLScriptElement> {
	const element = document.createElement('script') as HTMLScriptElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Section(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('section') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Select(debugName?: string): HTMLNode<HTMLSelectElement> {
	const element = document.createElement('select') as HTMLSelectElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Small(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('small') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Source(debugName?: string): HTMLNode<HTMLSourceElement> {
	const element = document.createElement('source') as HTMLSourceElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Span(debugName?: string): HTMLNode<HTMLSpanElement> {
	const element = document.createElement('span') as HTMLSpanElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Strong(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('strong') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Style(debugName?: string): HTMLNode<HTMLStyleElement> {
	const element = document.createElement('style') as HTMLStyleElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Sub(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('sub') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Summary(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('summary') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Sup(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('sup') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Table(debugName?: string): HTMLNode<HTMLTableElement> {
	const element = document.createElement('table') as HTMLTableElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Tbody(debugName?: string): HTMLNode<HTMLTableSectionElement> {
	const element = document.createElement('tbody') as HTMLTableSectionElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Td(debugName?: string): HTMLNode<HTMLTableCellElement> {
	const element = document.createElement('td') as HTMLTableCellElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Template(debugName?: string): HTMLNode<HTMLTemplateElement> {
	const element = document.createElement('template') as HTMLTemplateElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Textarea(debugName?: string): HTMLNode<HTMLTextAreaElement> {
	const element = document.createElement('textarea') as HTMLTextAreaElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Tfoot(debugName?: string): HTMLNode<HTMLTableSectionElement> {
	const element = document.createElement('tfoot') as HTMLTableSectionElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Th(debugName?: string): HTMLNode<HTMLTableCellElement> {
	const element = document.createElement('th') as HTMLTableCellElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Thead(debugName?: string): HTMLNode<HTMLTableSectionElement> {
	const element = document.createElement('thead') as HTMLTableSectionElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Time(debugName?: string): HTMLNode<HTMLTimeElement> {
	const element = document.createElement('time') as HTMLTimeElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Title(debugName?: string): HTMLNode<HTMLTitleElement> {
	const element = document.createElement('title') as HTMLTitleElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Tr(debugName?: string): HTMLNode<HTMLTableRowElement> {
	const element = document.createElement('tr') as HTMLTableRowElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Track(debugName?: string): HTMLNode<HTMLTrackElement> {
	const element = document.createElement('track') as HTMLTrackElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function U(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('u') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Ul(debugName?: string): HTMLNode<HTMLUListElement> {
	const element = document.createElement('ul') as HTMLUListElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Var(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('var') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Video(debugName?: string): HTMLNode<HTMLVideoElement> {
	const element = document.createElement('video') as HTMLVideoElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function Wbr(debugName?: string): HTMLNode<HTMLElement> {
	const element = document.createElement('wbr') as HTMLElement;
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}


export function PasswordInput(debugName?: string): HTMLNode<HTMLInputElement> {
	const element = document.createElement('input') as HTMLInputElement;
	element.type = 'password';
	element.autocomplete = 'new-password';
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}

export function TextInput(debugName?: string): HTMLNode<HTMLInputElement> {
	const element = document.createElement('input') as HTMLInputElement;
	element.type = 'text';
	element.autocomplete = 'off';
	if (debugName) element.setAttribute('data-debug-name', debugName);
	return new HTMLNode(element);
}
