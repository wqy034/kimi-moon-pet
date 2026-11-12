// 原生轮廓与呈现帧串行提交；不读取或模拟鼠标。
function silhouette(data) {
  const w=160,h=180, cells=new Uint8Array(w*h), expanded=new Uint8Array(w*h);
  for(let y=0;y<360;y++) for(let x=0;x<320;x++)
    if(data[(y*320+x)*4+3]) cells[(y>>1)*w+(x>>1)]=1;
  for(let y=0;y<h;y++) for(let x=0;x<w;x++) if(cells[y*w+x])
    for(let yy=Math.max(0,y-1);yy<=Math.min(h-1,y+1);yy++)
      expanded.fill(1,yy*w+Math.max(0,x-1),yy*w+Math.min(w,x+2));
  const rects=[];
  for(let y=0;y<h;y++) for(let x=0;x<w;) {
    if(!expanded[y*w+x]) {x++;continue;}
    const start=x; while(x<w&&expanded[y*w+x]) x++;
    rects.push([start*2,y*2,x*2,(y+1)*2]);
  }
  if(!rects.length) throw new Error('empty silhouette');
  return rects;
}
export class PetShape {
  constructor(source,target,invoke,presented,log){
    Object.assign(this,{source,target:target.getContext('2d'),invoke,presented,log,session:null,serial:0,key:null,pending:null,running:false,retryAt:0,failed:false});
  }
  offer(sprite){
    try {this.pending={pixels:this.source.getImageData(0,0,320,360),sprite};void this.pump();}
    catch(_){this.pending={pixels:null,sprite};void this.pump();}
  }
  paint(frame){this.target.putImageData(frame.pixels,0,0);this.presented(frame.sprite);}
  async pump(){
    if(this.running)return;this.running=true;
    try {while(this.pending){
      const frame=this.pending;this.pending=null;
      if(!frame.pixels){
        if(!this.failed)this.log('pet-shape stage=pixels-unavailable');this.failed=true;
        if(this.session!==null)await this.invoke('pet_shape_reset',{session:this.session}).catch(()=>{});
        this.session=null;this.key=null;this.target.clearRect(0,0,320,360);
        this.target.drawImage(this.source.canvas,0,0);this.presented(frame.sprite);continue;
      }
      if(performance.now()<this.retryAt){this.paint(frame);continue;}
      try{
        if(this.session===null)this.session=await this.invoke('pet_shape_begin');
        const rects=silhouette(frame.pixels.data),key=JSON.stringify(rects);
        if(key===this.key){this.paint(frame);continue;}
        const serial=++this.serial;
        if(!await this.invoke('pet_shape_prepare',{session:this.session,serial,rects}))throw new Error('stale');
        this.paint(frame);
        await new Promise(resolve=>requestAnimationFrame(resolve));
        if(!await this.invoke('pet_shape_commit',{session:this.session,serial}))throw new Error('stale');
        this.key=key;if(this.failed)this.log('pet-shape stage=recovered');else if(serial===1)this.log('pet-shape stage=ready');this.failed=false;
      }catch(_){
        if(!this.failed)this.log('pet-shape stage=apply-failed');this.failed=true;
        if(this.session!==null)await this.invoke('pet_shape_reset',{session:this.session}).catch(()=>{});
        this.session=null;this.key=null;this.retryAt=performance.now()+5000;this.paint(frame);
      }
    }}finally{this.running=false;}
  }
}
