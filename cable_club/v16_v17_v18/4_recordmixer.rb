#===============================================================================
# HandlerHashBasic from v20.1
#===============================================================================
class HandlerHashBasic
  def initialize
    @hash   = {}
    @addIfs = []
  end

  def [](entry)
    ret = nil
    ret = @hash[entry] if entry && @hash[entry]
    unless ret
      @addIfs.each do |addif|
        return addif[1] if addif[0].call(entry)
      end
    end
    return ret
  end

  def add(entry, handler = nil, &handlerBlock)
    if ![Proc, Hash].include?(handler.class) && !block_given?
      raise ArgumentError, "#{self.class.name} for #{entry.inspect} has no valid handler (#{handler.inspect} was given)"
    end
    return if !entry || (!(entry.is_a?(Symbol)) && entry.empty?)
    @hash[entry] = handler || handlerBlock
  end

  def addIf(conditionProc, handler = nil, &handlerBlock)
    if ![Proc, Hash].include?(handler.class) && !block_given?
      raise ArgumentError, "addIf call for #{self.class.name} has no valid handler (#{handler.inspect} was given)"
    end
    @addIfs.push([conditionProc, handler || handlerBlock])
  end

  def copy(src, *dests)
    handler = self[src]
    return if !handler
    dests.each { |dest| add(dest, handler) }
  end

  def remove(key)
    @hash.delete(key)
  end

  def clear
    @hash.clear
  end

  def each
    @hash.each_pair { |key, value| yield key, value }
  end

  def keys
    return @hash.keys.clone
  end

  def trigger(entry, *args)
    handler = self[entry]
    return handler ? handler.call(*args) : nil
  end
end

module RecordMixer
  @@records = HandlerHashBasic.new

  def self.copy(sym, *syms)
    @@records.copy(sym, *syms)
  end

  def self.register(sym, hash)
    @@records.add(sym, hash)
  end
  
  def self.each
    @@records.keys.each { |sym| yield sym }
  end

  def self.record_name(sym)
    return self.call("name", sym)
  end
  
  def self.prepare_record(sym)
    self.call("prepareData", sym)
  end
  
  def self.write_record(sym, writer)
    self.call("writeData", sym, writer)
  end
  
  def self.parse_record(sym, record)
    self.call("parseData", sym, record)
  end
  
  def self.finalize_record(sym)
    self.call("finalizeData", sym)
  end
    
  def self.call(func, sym, *args)
    r = @@records[sym]
    return nil if !r || !r[func]
    return r[func].call(*args)
  end
end

module CableClub
  def self.do_mix_records(connection)
    RecordMixer.each do |sym|
      record_name = RecordMixer.record_name(sym)
      yield _INTL("Preparing {1} Data",record_name) if block_given?
      RecordMixer.prepare_record(sym)
    end
    
    RecordMixer.each do |sym|
      record_name = RecordMixer.record_name(sym)
      yield _INTL("Sending {1} Data",record_name) if block_given?
      connection.send do |writer|
        writer.sym(sym.to_sym)
        RecordMixer.write_record(sym, writer)
      end
    end
    
    RecordMixer.each do |sym|
      record_name = RecordMixer.record_name(sym)
      ret = false
      loop do
        yield _INTL("Receiving {1} Data",record_name) if block_given?
        Graphics.update
        Input.update
        connection.update do |record|
          case (type = record.sym)
          when sym.to_sym
            RecordMixer.parse_record(sym, record)
            ret = true
          else
            raise "Unknown message: #{type}"
          end
        end
        break if ret
      end
    end
    
    RecordMixer.each do |sym|
      record_name = RecordMixer.record_name(sym)
      yield _INTL("Processing {1} Data",record_name) if block_given?
      RecordMixer.finalize_record(sym)
    end
  end
end