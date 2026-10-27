# SPDX-License-Identifier: GPL-2.0-or-later
# Ruby Marshal 4.8 used by RPG Maker archives and saves. Unsupported records
# fail explicitly rather than silently replacing game data.
module Marshal
  MAJOR_VERSION=4; MINOR_VERSION=8
  RestoreError=TypeError unless defined?(RestoreError)
  def self.load(source,callback=nil)
    source=source.read if source.respond_to?(:read)
    Reader.new(source,callback).load
  end
  class << self; alias restore load; end
  def self.dump(object,port=nil,limit=-1)
    limit=port if port.is_a?(Integer)
    bytes=Writer.new(limit).dump(object)
    port && !port.is_a?(Integer) ? (port.write(bytes); port) : bytes
  end
  class Reader
    def initialize(source,callback)
      raise TypeError,'instance of IO needed' unless source.is_a?(String)
      # SabiRuby's byteslice copies the whole receiver on every call; index the
      # binary string instead, which is O(slice length) like CRuby.
      @source=source.dup
      @source.force_encoding(Encoding::ASCII_8BIT) if @source.respond_to?(:force_encoding)
      @pos=0; @objects=[]; @symbols=[]; @callback=callback
    end
    def bytes(n)
      raise ArgumentError,'marshal data too short' if n<0 || @pos+n>@source.bytesize
      result=@source[@pos,n]; @pos+=n; result
    end
    def byte
      value=@source.getbyte(@pos)
      raise ArgumentError,'marshal data too short' if value.nil?
      @pos+=1
      value
    end
    def integer
      n=byte; n-=256 if n>=128
      return 0 if n==0
      return n-5 if n>4
      return n+5 if n < -4
      count=n.abs; value=0
      count.times { |i| value |= byte << (8*i) }
      value -= 1 << (8*count) if n<0
      value
    end
    def text; bytes(integer); end
    def klass(name); name.to_s.split('::').inject(Object) { |base,n| base.const_get(n) }; end
    def remember(obj); @objects << obj; obj; end
    def load
      raise TypeError,'incompatible marshal file format' unless byte==4 && byte<=8
      object
    end
    def object
      tag=byte
      result=case tag
      when 48 then nil
      when 84 then true
      when 70 then false
      when 105 then integer
      when 58 then s=text.to_sym; @symbols << s; s
      when 59
        i=integer; raise ArgumentError,'bad symbol link' unless i>=0 && i<@symbols.size; @symbols[i]
      when 64
        i=integer; raise ArgumentError,'bad object link' unless i>=0 && i<@objects.size; return @objects[i]
      when 34 then remember(defined?(FANPLAY) ? FANPLAY.force_utf8(text) : text)
      when 91
        a=remember([]); n=integer; raise ArgumentError,'negative array size' if n<0
        n.times { a << object }; a
      when 123,125
        h=remember({}); n=integer; raise ArgumentError,'negative hash size' if n<0
        n.times { k=object; h[k]=object }; h.default=object if tag==125; h
      when 111
        obj=remember(klass(object).allocate)
        integer.times { name=object; obj.instance_variable_set(name,object) }; obj
      when 117
        type=klass(object); payload=text
        remember(type._load(payload))
      when 85
        obj=remember(klass(object).allocate); obj.marshal_load(object); obj
      when 73
        # For I/u, CRuby registers the loaded object AFTER the payload's
        # attributes. Registering it first corrupts subsequent object links
        # whenever Time's zone/submicro attributes contain strings.
        user_record=@source.getbyte(@pos)==117
        if user_record
          byte
          type=klass(object); payload=text
        else
          obj=object
        end
        attributes={}
        integer.times do
          name=object; value=object
          attributes[name]=value
          if user_record
            payload.instance_variable_set(name,value) if name.to_s.start_with?('@')
          elsif name.to_s.start_with?('@')
            obj.instance_variable_set(name,value)
          elsif obj.is_a?(String) && name==:E
            obj.force_encoding(value ? Encoding::UTF_8 : Encoding::US_ASCII)
          elsif obj.is_a?(String) && name==:encoding
            obj.force_encoding(value)
          end
        end
        obj=remember(type._load(payload)) if user_record
        if user_record
          attributes.each { |name,value| obj.instance_variable_set(name,value) if name.to_s.start_with?('@') }
        end
        obj.__fanplay_restore_time_metadata(attributes) if obj.is_a?(Time)
        obj
      when 102
        s=text.split("\0")[0]
        value=case s; when 'nan' then 0.0/0.0; when 'inf' then 1.0/0.0; when '-inf' then -1.0/0.0; else s.to_f; end
        remember(value)
      when 108
        negative=byte==45; n=integer; value=0
        (n*2).times { |i| value |= byte << (8*i) }; remember(negative ? -value : value)
      when 47
        pattern=text; options=byte; remember(Regexp.new(pattern,options))
      when 99,109 then remember(klass(text))
      when 67
        # `C` wraps a subclass of Array/String/Hash with its base data. RGSS
        # animation containers are now plain objects, but old saves/data still
        # carry this record; allocate the class and let the surrounding `I`
        # record install the instance variables.
        type=klass(object); base=object
        obj=type.allocate
        if obj.is_a?(Array) && base.is_a?(Array)
          obj.replace(base)
        elsif obj.is_a?(String) && base.is_a?(String)
          obj.replace(base)
        elsif obj.is_a?(Hash) && base.is_a?(Hash)
          obj.replace(base)
        end
        remember(obj)
      when 83
        type=klass(object); obj=remember(type.new)
        integer.times { name=object; obj[name]=object }; obj
      else
        raise TypeError,"unsupported Marshal record #{tag}"
      end
      @callback ? @callback.call(result) : result
    end
  end
  class Writer
    def initialize(limit)
      @limit=limit; @out=[4,8].pack('C*'); @objects={}; @symbols={}
    end
    def dump(obj); object(obj,0); @out; end
    def byte(n); @out << [n & 255].pack('C'); end
    def integer(n)
      if n==0; byte(0)
      elsif n>0 && n<123; byte(n+5)
      elsif n<0 && n > -124; byte(n-5)
      else
        data=[]; value=n
        loop do
          data << (value & 255); value >>= 8
          break if value==0 || value == -1
        end
        byte(n<0 ? -data.size : data.size); data.each { |b| byte(b) }
      end
    end
    def text(s); integer(s.bytesize); @out << s.b; end
    def symbol(s)
      if @symbols.key?(s); byte(59); integer(@symbols[s])
      else; @symbols[s]=@symbols.size; byte(58); text(s.to_s)
      end
    end
    def object(obj,depth)
      raise ArgumentError,'exceed depth limit' if @limit>=0 && depth>=@limit
      case obj
      when NilClass; byte(48); return
      when TrueClass; byte(84); return
      when FalseClass; byte(70); return
      when Symbol; symbol(obj); return
      when Integer
        if obj>=-(1<<30) && obj<(1<<30); byte(105); integer(obj); return; end
      end
      id=obj.object_id
      if @objects.key?(id); byte(64); integer(@objects[id]); return; end
      if obj.is_a?(Time)
        byte(73); byte(117); symbol(obj.class.name.to_sym); text(obj._dump(@limit))
        attrs=obj.__fanplay_time_metadata
        integer(attrs.size); attrs.each { |name,value| symbol(name); object(value,depth+1) }
        @objects[id]=@objects.size
        return
      end
      @objects[id]=@objects.size
      if obj.respond_to?(:marshal_dump)
        byte(85); symbol(obj.class.name.to_sym); object(obj.marshal_dump,depth+1)
      elsif obj.respond_to?(:_dump)
        byte(117); symbol(obj.class.name.to_sym); text(obj._dump(@limit))
      else
        case obj
        when String
          attrs={}
          encoding=obj.encoding.name
          if encoding=='UTF-8' || encoding=='US-ASCII'
            attrs[:E]=encoding=='UTF-8'
          elsif encoding!='ASCII-8BIT'
            attrs[:encoding]=encoding
          end
          obj.instance_variables.each do |name|
            attrs[name]=obj.instance_variable_get(name) unless name.to_s.start_with?('@__sabi_')
          end
          byte(73) unless attrs.empty?
          byte(34); text(obj)
          unless attrs.empty?
            integer(attrs.size); attrs.each { |name,value| symbol(name); object(value,depth+1) }
          end
        when Array
          byte(91); integer(obj.size); obj.each { |v| object(v,depth+1) }
        when Hash
          byte(obj.default.nil? ? 123 : 125); integer(obj.size)
          obj.each { |k,v| object(k,depth+1); object(v,depth+1) }
          object(obj.default,depth+1) unless obj.default.nil?
        when Float
          byte(102); text(obj.nan? ? 'nan' : (obj.infinite? ? (obj<0 ? '-inf' : 'inf') : obj.to_s))
        when Integer
          byte(108); byte(obj<0 ? 45 : 43); data=[]; n=obj.abs
          while n>0; data << (n & 255); n >>= 8; end
          data << 0 if data.size.odd?; integer(data.size/2); data.each { |b| byte(b) }
        when Regexp
          byte(47); text(obj.source); byte(obj.options)
        when Class
          byte(99); text(obj.name)
        when Module
          byte(109); text(obj.name)
        when Struct
          byte(83); symbol(obj.class.name.to_sym); integer(obj.members.size)
          obj.each_pair { |name,value| symbol(name.to_sym); object(value,depth+1) }
        else
          raise TypeError,"can't dump #{obj.class}" if obj.class.name.nil?
          byte(111); symbol(obj.class.name.to_sym)
          names=obj.instance_variables.reject { |n| n.to_s.start_with?('@__sabi_') }
          integer(names.size); names.each { |n| symbol(n); object(obj.instance_variable_get(n),depth+1) }
        end
      end
    end
  end
end

# CRuby time.c's eight-byte Marshal payload uses UTC calendar fields even
# for local times. Preserve offset/zone metadata for CRuby round trips; the
# VM's existing local-time display remains UTC (it has no time-zone database).
class Time
  def self._load(data)
    raise TypeError,'marshaled time format differs' unless data.bytesize==8
    p,s=data.unpack('V2')
    if (p & 0x80000000)==0
      return Time.at(p,s)
    end
    year=((p >> 14) & 0xffff)+1900
    time=Time.utc(year,((p >> 10) & 15)+1,(p >> 5) & 31,p & 31,
                  (s >> 26) & 63,(s >> 20) & 63,s & 0xfffff)
    time.localtime if (p & 0x40000000)==0
    time
  end
  def _dump(_limit=-1)
    time=getutc
    raise RangeError,'time year outside supported Marshal range' unless (1900..67435).include?(time.year)
    p=0x80000000 | (utc? ? 0x40000000 : 0) | ((time.year-1900) << 14) |
      ((time.month-1) << 10) | (time.day << 5) | time.hour
    s=(time.min << 26) | (time.sec << 20) | (nsec / 1000)
    [p,s].pack('V2')
  end
  def __fanplay_restore_time_metadata(attrs)
    @__sabi_time_metadata=attrs.reject { |name,_| name.to_s.start_with?('@') }
    extra=attrs[:nano_num]
    if extra
      denominator=attrs[:nano_den] || 1
      raise TypeError,'unsupported fractional nanoseconds' unless denominator==1
      __sabi_time_nsec(self,(nsec / 1000)*1000+extra)
    elsif attrs[:submicro]
      bytes=attrs[:submicro].bytes
      extra=(bytes[0] >> 4)*100+(bytes[0] & 15)*10+((bytes[1] || 0) >> 4)
      __sabi_time_nsec(self,(nsec / 1000)*1000+extra)
    end
    self
  end
  def __fanplay_time_metadata
    attrs=(@__sabi_time_metadata || {}).dup
    attrs.delete(:submicro); attrs.delete(:nano_num); attrs.delete(:nano_den)
    extra=nsec % 1000
    if extra!=0
      attrs[:nano_num]=extra; attrs[:nano_den]=1
    end
    unless utc?
      attrs[:offset]=utc_offset unless attrs.key?(:offset)
      attrs[:zone]=zone unless attrs.key?(:zone)
    end
    instance_variables.each do |name|
      attrs[name]=instance_variable_get(name) unless name.to_s.start_with?('@__sabi_')
    end
    attrs
  end
end
