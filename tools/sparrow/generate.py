"""Generate golden TSV using unmodified decoder/collection method bodies from FNF's pinned Flixel.
Requires Python 3 and Haxe. Only tiny geometry/resource stubs replace the engine runtime.
"""
from pathlib import Path
import urllib.request, tempfile, subprocess, random
ROOT = Path(__file__).resolve().parents[2]
REV = '141f23c400c0508c76d5a09a143f5ce6790f8122'
BASE = f'https://raw.githubusercontent.com/FunkinCrew/flixel/{REV}/flixel/graphics/frames/'
def fetch(name):
    return urllib.request.urlopen(BASE+name+'.hx').read().decode()
def method(text, name):
    start=text.index('function '+name+'(')
    brace=text.index('{', start); depth=1; end=brace+1
    while depth:
        depth += (text[end]=='{') - (text[end]=='}'); end+=1
    return text[start:end]
atlas=fetch('FlxAtlasFrames'); collection=fetch('FlxFramesCollection')
decode=method(atlas,'fromSparrow')
decode=decode.replace('source:FlxGraphicAsset, xml:FlxXmlAsset','source:FlxGraphic, xml:Xml').replace('xml.getXml()','xml')
methods='\n'.join('public '+method(collection,n) for n in ['addAtlasFrame','checkFrame','addEmptyFrame','pushFrame'])
harness='''import haxe.xml.Access;
class Oracle {
 static function main() {
  var args=Sys.args();
  var frames=FlxAtlasFrames.fromSparrow(new FlxGraphic(128,96),Xml.parse(sys.io.File.getContent(args[0])));
  var out=new StringBuf();
  for(f in frames.frames) out.add(([f.name,f.frame.x,f.frame.y,f.frame.width,f.frame.height,f.sourceSize.x,f.sourceSize.y,f.offset.x,f.offset.y,f.angle,f.flipX,f.flipY,f.type==1]:Array<Dynamic>).join("\\t")+"\\n");
  sys.io.File.saveContent(args[1],out.toString());
 }
}
class FlxGraphic { public var width:Int; public var height:Int; public function new(w,h) {width=w;height=h;} }
class FlxG { public static var bitmap={add:function(g:FlxGraphic) return g}; public static var log={warn:function(s:String) {}}; }
class FlxMath { public static function bound(v:Float,a:Float,b:Float) return Math.max(a,Math.min(b,v)); }
class FlxDestroyUtil { public static function put<T>(v:T):T return null; }
class FlxPoint {
 public var x:Float; public var y:Float;
 public function new(x=0.,y=0.) {set(x,y);}
 public static function get(x=0.,y=0.) return new FlxPoint(x,y);
 public function set(x:Float,y:Float) {this.x=x;this.y=y;return this;}
 public function copyFrom(p:FlxPoint) return set(p.x,p.y);
}
class FlxRect {
 public var x:Float; public var y:Float; public var width:Float; public var height:Float;
 public var right(get,never):Float; function get_right() return x+width;
 public var bottom(get,never):Float; function get_bottom() return y+height;
 public function new(x=0.,y=0.,w=0.,h=0.) {set(x,y,w,h);}
 public static function get(x=0.,y=0.,w=0.,h=0.) return new FlxRect(x,y,w,h);
 public function set(x:Float,y:Float,w:Float,h:Float) {this.x=x;this.y=y;width=w;height=h;return this;}
 public function setSize(w:Float,h:Float) {width=w;height=h;}
}
enum abstract FlxFrameAngle(Int) from Int to Int { var ANGLE_0=0; var ANGLE_NEG_90=-90; }
class FlxFrameType {public static var EMPTY=1;}
class FlxFrame {
 public var name:String; public var frame:FlxRect; public var sourceSize=FlxPoint.get(); public var offset=FlxPoint.get();
 public var angle:FlxFrameAngle; public var flipX:Bool; public var flipY:Bool; public var type=0;
 public function new(p:FlxGraphic,a:FlxFrameAngle=0,x=false,y=false,d=0.) {angle=a;flipX=x;flipY=y;}
 public function cacheFrameMatrix() {}
}
class FlxAtlasFrames {
 public var parent:FlxGraphic; public var frames:Array<FlxFrame>=[]; public var framesByName:Map<String,FlxFrame>=[];
 public function new(p) {parent=p;}
 public static function findFrame(p:FlxGraphic):FlxAtlasFrames return null;
 public function exists(n:String) return framesByName.exists(n);
 public function getByName(n:String) return framesByName.get(n);
'''+ 'public static '+decode+'\n'+methods+'\n}\n'
rng=random.Random(4242)
rows=['<?xml version="1.0"?><TextureAtlas imagePath="atlas.png">']
for i in range(512):
    x=rng.randint(-20,150)+(.25 if i%7==0 else 0);y=rng.randint(-20,100)
    w=rng.randint(0,50);h=rng.randint(0,40)
    attrs=f'name="case{i:04}" x="{x}" y="{y}" width="{w}" height="{h}" rotated="{str(i%2==0).lower()}" flipX="{str(i%3==0).lower()}" flipY="{str(i%5==0).lower()}"'
    if i%4<2: attrs+=f' frameX="{-rng.randint(-5,30)}" frameY="{-rng.randint(-5,30)}" frameWidth="80" frameHeight="70"'
    rows.append('<SubTexture '+attrs+'/>')
rows.extend(['<SubTexture name="case0000" x="1" y="1" width="2" height="3"/>','<SubTexture name="empty" x="3" y="4" width="0" height="4"/>','<SubTexture name="empty" x="1" y="2" width="3" height="4"/>','<SubTexture name="escaped &amp; &#x41;" x="1.5" y="2.25" width="3.5" height="4.75"/>','</TextureAtlas>'])
xml=ROOT/'tests/fixtures/sparrow.xml';xml.write_text('\n'.join(rows))
with tempfile.TemporaryDirectory(prefix='velocity-sparrow-') as directory:
    Path(directory,'Oracle.hx').write_text(harness)
    subprocess.run(['haxe','-cp',directory,'--run','Oracle',str(xml),str(ROOT/'tests/fixtures/sparrow.tsv')],check=True)
print('Generated', len((ROOT/'tests/fixtures/sparrow.tsv').read_text().splitlines()), 'golden frames from FNF Flixel',REV)
