// Loads CSS locally from an official installation; never redistributes the vendor bundle.
import fs from 'node:fs';
import process from 'node:process';
import {Buffer} from 'node:buffer';
export function officialCss() {
  const filename=process.env.WORKBUDDY_556_ASAR||'.qa/workbuddy-5.5.6/application/resources/app.asar';
  const fd=fs.openSync(filename,'r');
  try {
    const prefix=Buffer.alloc(16);fs.readSync(fd,prefix,0,16,0);
    const length=prefix.readUInt32LE(12);if(length>32*1024*1024)throw Error('Invalid ASAR header');
    const header=Buffer.alloc(length);fs.readSync(fd,header,0,length,16);
    const assets=JSON.parse(header.toString()).files.renderer.files.assets.files;
    return ['index-','home-','main-content-core-','lib-chat-ui-','ui-docs-viewer-'].map(part=>{
      const name=Object.keys(assets).find(n=>n.startsWith(part)&&n.endsWith('.css'));
      if(!name)throw Error('Missing official CSS: '+part);
      const item=assets[name];if(item.size>20*1024*1024)throw Error('CSS limit');
      const buffer=Buffer.alloc(item.size);fs.readSync(fd,buffer,0,item.size,8+prefix.readUInt32LE(4)+Number(item.offset));
      return buffer.toString();
    }).join('\n');
  }finally{fs.closeSync(fd);}
}
