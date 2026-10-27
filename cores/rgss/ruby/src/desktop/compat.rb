# SPDX-License-Identifier: GPL-2.0-or-later
# Host services absent from mruby's default library. No CRuby is loaded.
$LOADED_FEATURES.concat(['zlib','json','weakref','fileutils'])
RUBY_PLATFORM = __sabi_platform unless defined?(RUBY_PLATFORM)
TOPLEVEL_BINDING = binding unless defined?(TOPLEVEL_BINDING)
class SystemCallError < StandardError; end
class IOError < StandardError; end
class EOFError < IOError; end
class SystemExit < Exception
  attr_reader :status
  def initialize(status=0); @status=status; super('exit'); end
end
module Kernel
  def exit(status=0); raise SystemExit.new(status); end
  def print(*args); args.each { |a| __sabi_stderr(a.to_s) }; nil; end
  def puts(*args); args.each { |a| a.is_a?(Array) ? puts(*a) : __sabi_stderr(a.to_s.chomp+"\n") }; nil; end
  module_function :print, :puts
end
class FanplayStderr
  def write(s); __sabi_stderr(s.to_s); end
  def puts(*s); Kernel.puts(*s); end
  def flush; self; end
end
STDERR = FanplayStderr.new
STDOUT = STDERR
$stderr = STDERR
$stdout = STDOUT
class Exception
  def full_message(highlight: false, order: :top)
    "#{self.class}: #{message}\n#{(backtrace || []).join("\n")}\n"
  end
end
class UnboundMethod
  def bind_call(obj,*args,**kwargs,&block); bind(obj).call(*args,**kwargs,&block); end
end
module ENV
  def self.[](key); __sabi_env(key.to_s); end
  def self.fetch(key,*fallback)
    value=self[key]
    return value unless value.nil?
    return yield(key) if block_given?
    return fallback[0] unless fallback.empty?
    raise KeyError, "key not found: #{key}"
  end
end
module Process
  CLOCK_MONOTONIC=1
  def self.clock_gettime(_clock); __sabi_clock; end
  def self.pid; __sabi_pid; end
end
# Execution is confined to the desktop game thread. Starting Ruby threads is
# deliberately unsupported; the legacy Thread.critical API uses this mutex.
class Mutex
  def initialize; @locked=false; end
  def try_lock; return false if @locked; @locked=true; end
  def lock; raise ThreadError,'recursive lock' if @locked; @locked=true; self; end
  def unlock; @locked=false; self; end
  def synchronize; lock; begin; yield; ensure; unlock; end; end
end
class ThreadError < StandardError; end
class Thread
  def self.current; @current ||= {}; end
  def self.new(*args); raise NotImplementedError,'SabiRuby RGSS does not support Ruby threads'; end
end
class FanplayWeakRef
  class RefError < StandardError; end
  def initialize(value)
    if value.nil? || value == true || value == false || value.is_a?(Numeric) || value.is_a?(Symbol)
      @immediate=[value]
    else
      @handle=__sabi_weak(value)
    end
  end
  def __getobj__
    return @immediate[0] if @immediate
    value=__sabi_weak_get(@handle)
    raise RefError,'Invalid Reference - probably recycled' if value.nil?
    value
  end
  def weakref_alive?; !!@immediate || !__sabi_weak_get(@handle).nil?; end
  def method_missing(name,*args,**kwargs,&block); __getobj__.__send__(name,*args,**kwargs,&block); end
  def respond_to_missing?(name,private=false); weakref_alive? && __getobj__.respond_to?(name,private); end
end
class WeakRef < FanplayWeakRef; end
class FanplayWeakRef
  def disposed?
    object=__getobj__
    object.respond_to?(:disposed?) ? object.disposed? : false
  rescue RefError
    true
  end
end
module FileTest
  def self.exist?(path); __sabi_exists(path.to_s); end
  def self.file?(path); __sabi_file(path.to_s); end
  def self.directory?(path); __sabi_directory(path.to_s); end
end
class File
  SEPARATOR='/'
  Stat=Struct.new(:size,:mtime,:ctime) do
    def ino; 0; end
  end
  def self.exist?(p); FileTest.exist?(p); end
  def self.file?(p); FileTest.file?(p); end
  def self.directory?(p); FileTest.directory?(p); end
  def self.binread(p); __sabi_read(p.to_s); end
  def self.read(p); binread(p); end
  def self.binwrite(p,s); __sabi_write(p.to_s,s); end
  def self.write(p,s); binwrite(p,s); end
  def self.delete(p); __sabi_delete(p.to_s); 1; end
  def self.rename(a,b); __sabi_rename(a.to_s,b.to_s); 0; end
  def self.size(p); stat(p).size; end
  def self.mtime(p); stat(p).mtime; end
  def self.ctime(p); stat(p).ctime; end
  def self.stat(p)
    size,modified,created=__sabi_stat(p.to_s)
    stamp=lambda { |ns| __sabi_time_nsec(Time.at(ns / 1_000_000_000),ns % 1_000_000_000) }
    Stat.new(size,stamp.call(modified),stamp.call(created))
  end
  def self.join(*parts); parts.flatten.map(&:to_s).join('/').gsub(%r{/+},'/'); end
  def self.expand_path(path,base=Dir.pwd)
    path=path.to_s.tr('\\','/')
    path=join(base,path) unless path.start_with?('/') || path.match?(/\A[A-Za-z]:\//)
    prefix=path.start_with?('/') ? '/' : ''
    result=[]
    path.split('/').each { |part| part=='..' ? result.pop : (result << part unless part.empty? || part=='.') }
    prefix+result.join('/')
  end
  def self.basename(path,suffix=nil)
    name=path.to_s.tr('\\','/').split('/').last || ''
    suffix && name.end_with?(suffix) ? name[0,name.size-suffix.size] : name
  end
  def self.dirname(path)
    s=path.to_s.tr('\\','/'); i=s.rindex('/'); i ? (i==0 ? '/' : s[0,i]) : '.'
  end
  def self.extname(path)
    name=basename(path); i=name.rindex('.'); i && i>0 ? name[i..-1] : ''
  end
  def self.open(path,mode='r',**_options)
    f=new(path,mode)
    return f unless block_given?
    begin; yield f; ensure; f.close; end
  end
  def self.foreach(path,&block); open(path) { |f| f.each_line(&block) }; end
  def initialize(path,mode='r')
    @path=path.to_s; @token=__sabi_open(@path,mode); @closed=false
  end
  attr_reader :path
  def read(n=nil,out=nil)
    raise IOError,'closed stream' if @closed
    raise ArgumentError,'negative length' if n && n<0
    s=__sabi_io(@token,'read','',n || -1)
    out.replace(s || '') if out
    s.nil? ? nil : (out || s)
  end
  def write(s); __sabi_io(@token,'write',s.to_s,0); end
  def <<(s); write(s); self; end
  def close; __sabi_io(@token,'close','',0) unless @closed; @closed=true; nil; end
  def closed?; @closed; end
  def pos; __sabi_io(@token,'pos','',0); end
  def size; __sabi_io(@token,'size','',0); end
  def seek(offset,whence=0)
    offset += pos if whence==1
    offset += size if whence==2
    raise ArgumentError,'invalid whence' unless [0,1,2].include?(whence)
    __sabi_io(@token,'seek','',offset)
  end
  def pos=(n); seek(n); n; end
  def rewind; seek(0); end
  def eof?; pos>=size; end
  alias eof eof?
  def getc; read(1); end
  def getbyte; s=read(1); s && s.getbyte(0); end
  def binmode; self; end
  def flush; self; end
  def gets(separator="\n")
    return nil if eof?
    line=''
    while (c=read(1)); line << c; break if line.end_with?(separator); end
    line
  end
  def each_line
    return enum_for(:each_line) unless block_given?
    while (line=gets); yield line; end
    self
  end
end
class Dir
  def self.pwd; __sabi_pwd; end
  def self.exist?(p); __sabi_directory(p.to_s); end
  def self.children(p); __sabi_children(p.to_s); end
  def self.entries(p); ['.','..']+children(p); end
  def self.glob(pattern)
    values=__sabi_glob(pattern.to_s).sort
    block_given? ? values.each { |p| yield p } : values
  end
  def self.[](pattern); glob(pattern); end
  def self.mkdir(p); __sabi_mkdir(p.to_s); 0; end
  def self.delete(p); __sabi_rmdir(p.to_s); 0; end
  def self.rmdir(p); delete(p); end
  def self.unlink(p); delete(p); end
  def self.foreach(p)
    return enum_for(:foreach,p) unless block_given?
    entries(p).each { |name| yield name }
    nil
  end
  def self.chdir(path=nil)
    path ||= __sabi_env('HOME') || '.'
    previous=pwd
    __sabi_chdir(path.to_s)
    return 0 unless block_given?
    begin
      yield path
    ensure
      __sabi_chdir(previous)
    end
  end
end
module FileUtils
  def self.mkdir_p(p); __sabi_mkdir(p.to_s); [p]; end
  def self.cp_r(src,dest)
    if File.directory?(src)
      mkdir_p(dest)
      Dir.children(src).each { |n| cp_r(File.join(src,n),File.join(dest,n)) }
    else
      File.binwrite(dest,File.binread(src))
    end
  end
end
module Zlib
  MAX_WBITS=15; DEFAULT_COMPRESSION=-1; FINISH=4
  class Error < StandardError; end
  class DataError < Error; end
  class Inflate
    def initialize(wbits=MAX_WBITS); @raw=wbits<0; @finished=false; end
    def inflate(data)
      result=__sabi_inflate(data,@raw); @finished=true; result
    rescue RuntimeError => e
      raise DataError,e.message
    end
    def finished?; @finished; end
    def finish; ''; end
    def close; nil; end
    def self.inflate(data); new.inflate(data); end
  end
  class Deflate
    def initialize(level=DEFAULT_COMPRESSION); @level=level; end
    def deflate(data,flush=FINISH)
      raise NotImplementedError,'streaming deflate is not supported' unless flush==FINISH
      __sabi_deflate(data,@level)
    end
    def close; nil; end
    def self.deflate(data,level=DEFAULT_COMPRESSION); new(level).deflate(data); end
  end
end

class EncodingError < StandardError; end
module Encoding
  # CRuby's `String#encoding` is an Encoding object whose `name` is the
  # canonical name. mruby has no such class, so the host provides one; RGSS
  # scripts compare `text.encoding.name` with "UTF-8".
  class Tag
    attr_reader :name
    def initialize(name); @name=name; end
    def to_s; @name; end
    def inspect; "#<Encoding:#{@name}>"; end
    def ==(other)
      other.respond_to?(:name) ? other.name==@name : other.to_s==@name
    end
    alias === ==
  end
  UTF_8=Tag.new('UTF-8'); BINARY=Tag.new('ASCII-8BIT'); ASCII_8BIT=BINARY; US_ASCII=Tag.new('US-ASCII')
  Windows_1252=Tag.new('Windows-1252'); ISO_8859_1=Tag.new('ISO-8859-1')
  def self.find(name)
    case name.to_s
    when 'UTF-8' then UTF_8
    when 'ASCII-8BIT', 'BINARY' then BINARY
    when 'US-ASCII' then US_ASCII
    when 'Windows-1252' then Windows_1252
    when 'ISO-8859-1' then ISO_8859_1
    else raise ArgumentError, "unknown encoding: #{name}"
    end
  end
end
class String
  def encoding
    name=__sabi_encoding(self)
    name ? Encoding.find(name) : (__sabi_is_binary(self) ? Encoding::BINARY : Encoding::UTF_8)
  end
  def force_encoding(name)
    __sabi_set_encoding(self,name.to_s)
    __sabi_binary(self,name.to_s==Encoding::BINARY.to_s)
    self
  end
  def valid_encoding?; encoding==Encoding::BINARY || __sabi_valid_utf8(self); end
  def encode(target,source=nil,invalid: nil,undef: nil,replace: nil)
    target=target.to_s
    return dup.force_encoding(target) if target==Encoding::BINARY.to_s
    raise EncodingError,'only UTF-8 output is supported' unless target==Encoding::UTF_8.to_s
    __sabi_transcode(self,(source || encoding).to_s,!invalid.nil?).force_encoding(target)
  rescue RuntimeError => e
    raise EncodingError,e.message
  end
end
class String
  alias __sabi_downcase downcase
  def downcase(*options)
    raise ArgumentError,'unsupported case option' unless options.empty? || options==[:fold]
    __sabi_downcase
  end
end
# The VM registers this primitive directly on Object, ahead of Kernel.
class Object
  def __exec_file(source,path)
    eval(source,TOPLEVEL_BINDING,path,1)
  end
  private :__exec_file
end
