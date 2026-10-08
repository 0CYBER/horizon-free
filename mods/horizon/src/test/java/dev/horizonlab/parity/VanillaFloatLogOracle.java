package dev.horizonlab.parity;

import java.io.BufferedOutputStream;
import net.minecraft.world.level.levelgen.densityfunction.*;
import net.minecraft.world.level.levelgen.densityfunction.op.UnaryFunction;

/** Original binary transport; all logarithms use vanilla's compiled sampler. */
public final class VanillaFloatLogOracle {
    public static void main(String[] args) throws Exception {
        long start=Long.parseLong(args[0]),remaining=Long.parseLong(args[1]);
        if(args.length!=2 || start<0 || remaining<0 || start+remaining>0x100000000L)
            throw new IllegalArgumentException("invalid float bit range");
        var sampler=new UnaryFunction.LogSampler(new DensitySampler() {
            public float sampleValue(SamplerContext context,int x,int y,int z) {return Float.intBitsToFloat(x);}
            public void sampleVolume(SamplerContext context,DensityBuffer buffer,DensityVolume volume) {throw new AssertionError("unexpected volume");}
        });
        byte[] buffer=new byte[1<<20];
        try(var out=new BufferedOutputStream(System.out,1<<20)) {
            while(remaining>0) {
                int count=(int)Math.min(remaining,buffer.length/4);
                for(int i=0;i<count;i++) {
                    int bits=Float.floatToIntBits(sampler.sampleValue(SamplerContext.EMPTY_UNCACHED,(int)(start+i),0,0));
                    int at=i*4;
                    buffer[at]=(byte)(bits>>>24);buffer[at+1]=(byte)(bits>>>16);
                    buffer[at+2]=(byte)(bits>>>8);buffer[at+3]=(byte)bits;
                }
                out.write(buffer,0,count*4);
                start+=count;remaining-=count;
            }
        }
    }
}
